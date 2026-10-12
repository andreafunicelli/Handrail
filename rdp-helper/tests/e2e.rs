//! The helper against a real RDP server (IronRDP's, in this process): NLA login, the first
//! desktop update arriving as tiles, a key press reaching the server, and a clean close.

use ironrdp_server::{
    BitmapUpdate, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat,
    RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, TlsIdentityCtx,
};
use std::num::{NonZeroU16, NonZeroUsize};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;

const W: u16 = 320;
const H: u16 = 200;

struct Desktop;

#[async_trait::async_trait]
impl RdpServerDisplay for Desktop {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: W,
            height: H,
        }
    }

    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(Updates { sent: false }))
    }
}

/// One full red frame, then nothing.
struct Updates {
    sent: bool,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        if self.sent {
            std::future::pending::<()>().await;
        }
        self.sent = true;
        let data: Vec<u8> = (0..usize::from(W) * usize::from(H))
            .flat_map(|_| [0u8, 0, 255, 255])
            .collect();
        Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: NonZeroU16::new(W).unwrap(),
            height: NonZeroU16::new(H).unwrap(),
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride: NonZeroUsize::new(usize::from(W) * 4).unwrap(),
        })))
    }
}

struct Keys(mpsc::UnboundedSender<KeyboardEvent>);

impl RdpServerInputHandler for Keys {
    fn keyboard(&mut self, event: KeyboardEvent) {
        let _ = self.0.send(event);
    }
    fn mouse(&mut self, _: MouseEvent) {}
}

async fn send(stdin: &mut ChildStdin, kind: u8, body: &[u8]) {
    stdin
        .write_all(&u32::try_from(body.len() + 1).unwrap().to_le_bytes())
        .await
        .unwrap();
    stdin.write_all(&[kind]).await.unwrap();
    stdin.write_all(body).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn recv(stdout: &mut ChildStdout) -> Option<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    stdout.read_exact(&mut len).await.ok()?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    stdout.read_exact(&mut buf).await.ok()?;
    let kind = buf.remove(0);
    Some((kind, buf))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connects_renders_types_and_closes() {
    // A self-signed certificate for the server's TLS (and so NLA).
    let dir = tempfile::tempdir().unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let (cert_path, key_path) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.signing_key.serialize_pem()).unwrap();
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).unwrap();

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (keys_tx, mut keys) = mpsc::unbounded_channel();
    // The server is not `Send`: it is built and run on a thread (and runtime) of its own.
    std::thread::spawn(move || {
        let mut server = RdpServer::builder()
            .with_addr(([127, 0, 0, 1], port))
            .with_hybrid(identity.make_acceptor().unwrap(), identity.pub_key.clone())
            .with_input_handler(Keys(keys_tx))
            .with_display_handler(Desktop)
            .build();
        server.set_credentials(Some(Credentials {
            username: "handrail".into(),
            password: "s3cret!".into(),
            domain: None,
        }));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(server.run());
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut helper = Command::new(env!("CARGO_BIN_EXE_handrail-rdp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = helper.stdin.take().unwrap();
    let mut stdout = helper.stdout.take().unwrap();
    let start = serde_json::json!({
        "host": "127.0.0.1", "port": port, "username": "handrail", "password": "s3cret!",
        "width": W, "height": H,
    });
    send(&mut stdin, 0x01, start.to_string().as_bytes()).await;

    // The red desktop arrives as tiles covering it.
    let tiles = timeout(Duration::from_secs(60), async {
        loop {
            match recv(&mut stdout).await {
                Some((0x11, body)) => break body,
                Some((0x13 | 0x14, body)) => {
                    panic!("session ended: {}", String::from_utf8_lossy(&body))
                }
                Some(_) => continue,
                None => panic!("helper exited"),
            }
        }
    })
    .await
    .expect("no desktop within 60 s");
    let u16_at = |i: usize| u16::from_le_bytes([tiles[i], tiles[i + 1]]);
    assert_eq!((u16_at(0), u16_at(2)), (W, H), "desktop size");
    assert!(u16_at(4) >= 1, "at least one tile");
    let first_pixel = &tiles[14..18];
    assert!(
        first_pixel[0] > 200 && first_pixel[1] < 60 && first_pixel[2] < 60,
        "red, got {first_pixel:?}"
    );

    // A key press ('A', scancode 0x1E) reaches the server.
    send(
        &mut stdin,
        0x02,
        br#"[{"t":"key","code":30,"down":true},{"t":"key","code":30,"down":false}]"#,
    )
    .await;
    let pressed = timeout(Duration::from_secs(10), async {
        loop {
            match keys.recv().await {
                Some(KeyboardEvent::Pressed { code, extended }) => break (code, extended),
                Some(_) => continue,
                None => panic!("server input closed"),
            }
        }
    })
    .await
    .expect("key never reached the server");
    assert_eq!(pressed, (0x1E, false));

    // Close: the helper signs off and exits.
    send(&mut stdin, 0x04, b"").await;
    let status = timeout(Duration::from_secs(15), helper.wait())
        .await
        .expect("helper did not exit")
        .unwrap();
    assert!(status.success(), "{status:?}");
}

#[tokio::test]
async fn reports_a_refused_connection() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut helper = Command::new(env!("CARGO_BIN_EXE_handrail-rdp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = helper.stdin.take().unwrap();
    let mut stdout = helper.stdout.take().unwrap();
    let start = serde_json::json!({
        "host": "127.0.0.1", "port": port, "username": "u", "password": "p", "width": 640, "height": 480,
    });
    send(&mut stdin, 0x01, start.to_string().as_bytes()).await;
    let (kind, body) = timeout(Duration::from_secs(30), recv(&mut stdout))
        .await
        .unwrap()
        .expect("a message");
    assert_eq!(kind, 0x13, "{}", String::from_utf8_lossy(&body));
    let msg: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(!msg["message"].as_str().unwrap().is_empty());
}
