//! `handrail-rdp`: one RDP session, driven by Handrail over stdin/stdout (see [`proto`]).
//!
//! Logs go to stderr (`HANDRAIL_RDP_LOG=debug` for more); stdout carries only protocol messages.

mod input;
mod proto;
mod tiles;

use ironrdp_client::config::{Config, ConfigBuilder, Destination};
use ironrdp_client::rdp::{RdpClient, RdpInputEvent, RdpOutputEvent};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use serde_json::json;
use std::process::ExitCode;
use tokio::io::{AsyncWrite, Stdin};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("HANDRAIL_RDP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("handrail-rdp: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::BufWriter::new(tokio::io::stdout());
    let config = match proto::read(&mut stdin).await? {
        Some((proto::START, body)) => {
            let start: proto::Start = serde_json::from_slice(&body)?;
            config(start)
        }
        _ => Err(anyhow::anyhow!("expected a start message")),
    };
    let config = match config {
        Ok(c) => c,
        Err(e) => {
            fail(&mut stdout, &e.to_string()).await?;
            return Err(e);
        }
    };

    let (tx, mut events) = mpsc::channel(4);
    let client = RdpClient::new(config, tx);
    let input = client.input_sender();
    // On a thread of its own, as IronRDP's viewer does: the session future is not `Send`.
    // Returning from `main` ends the process, and with it this thread.
    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(client.run()),
            Err(e) => tracing::error!("no runtime for the session: {e}"),
        }
    });
    tokio::spawn(forward_input(stdin, input));

    let mut differ = tiles::Differ::default();
    let mut result = Ok(());
    while let Some(event) = events.recv().await {
        match event {
            RdpOutputEvent::Image {
                buffer,
                width,
                height,
            } => {
                let (w, h) = (width.get(), height.get());
                let rects = differ.diff(&buffer, w, h);
                if let Some(payload) = tiles::encode(&buffer, w, &rects) {
                    proto::write(&mut stdout, tiles::KIND, &payload).await?;
                }
            }
            RdpOutputEvent::PointerDefault => {
                proto::write(&mut stdout, proto::POINTER, &[proto::POINTER_DEFAULT]).await?
            }
            RdpOutputEvent::PointerHidden => {
                proto::write(&mut stdout, proto::POINTER, &[proto::POINTER_HIDDEN]).await?
            }
            RdpOutputEvent::PointerPosition { .. } => {}
            RdpOutputEvent::PointerBitmap(p) => {
                let payload = proto::pointer_bitmap(
                    p.width,
                    p.height,
                    p.hotspot_x,
                    p.hotspot_y,
                    &p.bitmap_data,
                );
                proto::write(&mut stdout, proto::POINTER, &payload).await?;
            }
            RdpOutputEvent::ConnectionFailure(e) => {
                tracing::debug!(error = ?e, "connection failed");
                fail(&mut stdout, &e.to_string()).await?;
                result = Err(anyhow::anyhow!("connection failed: {e}"));
                break;
            }
            RdpOutputEvent::Terminated(r) => {
                let (reason, error) = match r {
                    Ok(reason) => (reason.to_string(), false),
                    Err(e) => (e.to_string(), true),
                };
                let body = json!({ "reason": reason, "error": error }).to_string();
                proto::write(&mut stdout, proto::TERMINATED, body.as_bytes()).await?;
                break;
            }
        }
    }
    result
}

async fn fail<W: AsyncWrite + Unpin>(out: &mut W, message: &str) -> std::io::Result<()> {
    proto::write(
        out,
        proto::ERROR,
        json!({ "message": message }).to_string().as_bytes(),
    )
    .await
}

/// Handrail's input, resize and close messages → the session. Stdin closing (Handrail gone)
/// ends the session too.
async fn forward_input(mut stdin: Stdin, tx: mpsc::UnboundedSender<RdpInputEvent>) {
    let mut db = ironrdp_input::Database::new();
    while let Ok(Some((kind, body))) = proto::read(&mut stdin).await {
        match kind {
            proto::INPUT => {
                let Ok(ops) = serde_json::from_slice::<Vec<proto::Input>>(&body) else {
                    tracing::warn!("ignoring malformed input");
                    continue;
                };
                for op in &ops {
                    let events = match input::operations(op) {
                        Some(ops) => db.apply(ops),
                        None => db.release_all(),
                    };
                    if !events.is_empty() && tx.send(RdpInputEvent::FastPath(events)).is_err() {
                        return;
                    }
                }
            }
            proto::RESIZE => {
                let Ok(r) = serde_json::from_slice::<proto::Resize>(&body) else {
                    continue;
                };
                let _ = tx.send(RdpInputEvent::Resize {
                    width: r.width,
                    height: r.height,
                    scale_factor: r.scale,
                    physical_size: None,
                });
            }
            proto::CLOSE => break,
            other => tracing::warn!(kind = other, "unknown message"),
        }
    }
    let _ = tx.send(RdpInputEvent::Close);
}

fn config(s: proto::Start) -> anyhow::Result<Config> {
    // One session per process, so the process-wide switch is this session's.
    ironrdp_connector::set_console_session(s.admin);
    let mut b = ConfigBuilder::new()
        .with_destination(Destination::from_parts(s.host, s.port))
        .with_username(s.username)
        .with_password(s.password)
        .with_desktop_width(s.width.max(200))
        .with_desktop_height(s.height.max(200))
        .with_desktop_scale_factor(s.scale)
        .with_client_build(client_build())
        .with_client_dir("C:\\Windows\\System32\\mstscax.dll")
        .with_platform(platform())
        .with_client_name(client_name());
    if let Some(d) = s.domain.filter(|d| !d.trim().is_empty()) {
        b = b.with_domain(d);
    }
    b.build()
}

/// 1.2.3 → 123, as other clients report it.
fn client_build() -> u32 {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let (a, b, c) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    a * 100 + b * 10 + c
}

/// The computer's name as the server sees it (15 characters at most, as in NetBIOS).
fn client_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "handrail".into());
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(15)
        .collect::<String>()
}

fn platform() -> MajorPlatformType {
    if cfg!(windows) {
        MajorPlatformType::WINDOWS
    } else if cfg!(target_os = "macos") {
        MajorPlatformType::MACINTOSH
    } else {
        MajorPlatformType::UNIX
    }
}
