//! Text clipboard for clients where IronRDP has no native clipboard (macOS, Linux).
//!
//! - Local → remote: Handrail sends what the user pasted (the webview's paste event, so no
//!   clipboard permission prompt) as a `clipboard` input; [`Clipboard::offer`] announces it and
//!   waits until the server took the announcement, then the paste keystroke follows.
//! - Remote → local: when the server announces a copy with text, the text is fetched right away
//!   and written to the system clipboard (`pbcopy`, `wl-copy`, `xclip` or `xsel`).
//!
//! On Windows IronRDP's own backend shares the system clipboard, and this one is not used.

use ironrdp_client::rdp::RdpInputEvent;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

/// Longest wait for the server to take an announcement before the paste goes ahead anyway.
const OFFER_WAIT: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
struct Shared {
    /// What the user last pasted into the session, served when the server asks for it.
    local: Mutex<Option<String>>,
    /// The server answered our latest format list.
    answered: Notify,
}

/// The helper's handle on the clipboard channel.
#[derive(Clone, Debug)]
pub struct Clipboard {
    shared: Arc<Shared>,
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl Clipboard {
    pub fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self {
            shared: Arc::default(),
            tx,
        }
    }

    pub fn factory(&self) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(Factory(self.clone()))
    }

    /// Announce `text` to the server, then wait (briefly) until it has taken the announcement,
    /// so that a paste keystroke sent next finds the text there.
    pub async fn offer(&self, text: String) {
        *self.shared.local.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        let answered = self.shared.answered.notified();
        let _ = self.tx.send(RdpInputEvent::Clipboard(
            ClipboardMessage::SendInitiateCopy(text_formats()),
        ));
        let _ = tokio::time::timeout(OFFER_WAIT, answered).await;
    }
}

fn text_formats() -> Vec<ClipboardFormat> {
    vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
}

struct Factory(Clipboard);

impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(Backend(self.0.clone()))
    }
}

#[derive(Debug)]
struct Backend(Clipboard);

ironrdp_core::impl_as_any!(Backend);

impl Backend {
    fn send(&self, message: ClipboardMessage) {
        let _ = self.0.tx.send(RdpInputEvent::Clipboard(message));
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    /// The channel starts (or the server asks again): announce what we hold, if anything.
    fn on_request_format_list(&mut self) {
        let has_text = self
            .0
            .shared
            .local
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let formats = if has_text { text_formats() } else { Vec::new() };
        self.send(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn on_format_list_response(&mut self, _ok: bool) {
        self.0.shared.answered.notify_waiters();
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    /// Something was copied on the server: fetch its text now, for the local clipboard.
    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        if formats
            .iter()
            .any(|f| f.id == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let local = self
            .0
            .shared
            .local
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let response: OwnedFormatDataResponse = match local {
            Some(text) if request.format == ClipboardFormatId::CF_UNICODETEXT => {
                FormatDataResponse::new_unicode_string(&text)
            }
            _ => FormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        match response.to_unicode_string() {
            Ok(text) => {
                let text = text.trim_end_matches('\0').to_owned();
                std::thread::spawn(move || {
                    if let Err(e) = write_system_clipboard(&text) {
                        tracing::warn!("could not set the local clipboard: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("undecodable clipboard text: {e}"),
        }
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _: LockDataId) {}

    fn on_unlock(&mut self, _: LockDataId) {}
}

/// The commands that set the system clipboard from stdin, in order of preference.
fn clipboard_commands() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "macos") {
        &[("/usr/bin/pbcopy", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    }
}

fn write_system_clipboard(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let mut last = std::io::Error::other("no clipboard command found");
    for (program, args) in clipboard_commands() {
        if *program == "wl-copy" && !wayland {
            continue;
        }
        let child = Command::new(program)
            .args(*args)
            // pbcopy reads stdin in the locale's encoding.
            .env("LANG", "en_US.UTF-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                last = e;
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if status.success() {
            return Ok(());
        }
        last = std::io::Error::other(format!("{program} exited with {status}"));
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> (Backend, mpsc::UnboundedReceiver<RdpInputEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Backend(Clipboard::new(tx)), rx)
    }

    fn message(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>) -> ClipboardMessage {
        match rx.try_recv() {
            Ok(RdpInputEvent::Clipboard(m)) => m,
            other => panic!("expected a clipboard message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn offers_pasted_text_and_serves_it() {
        let (mut b, mut rx) = backend();
        // Nothing pasted yet: an empty format list.
        b.on_request_format_list();
        assert!(matches!(message(&mut rx), ClipboardMessage::SendInitiateCopy(f) if f.is_empty()));

        let clip = b.0.clone();
        let offer = tokio::spawn(async move { clip.offer("héllo".into()).await });
        tokio::task::yield_now().await;
        assert!(matches!(
            message(&mut rx),
            ClipboardMessage::SendInitiateCopy(f) if f.len() == 1 && f[0].id == ClipboardFormatId::CF_UNICODETEXT
        ));
        b.on_format_list_response(true);
        tokio::time::timeout(Duration::from_millis(100), offer)
            .await
            .expect("offer returns once the server answered")
            .unwrap();

        b.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });
        let ClipboardMessage::SendFormatData(r) = message(&mut rx) else {
            panic!("expected data");
        };
        assert_eq!(
            r.to_unicode_string().unwrap().trim_end_matches('\0'),
            "héllo"
        );

        b.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_TEXT,
        });
        let ClipboardMessage::SendFormatData(r) = message(&mut rx) else {
            panic!("expected data");
        };
        assert!(r.is_error());
    }

    #[tokio::test]
    async fn offer_gives_up_waiting() {
        let (b, _rx) = backend();
        tokio::time::timeout(OFFER_WAIT * 2, b.0.offer("x".into()))
            .await
            .expect("offer does not hang without an answer");
    }

    #[test]
    fn fetches_remote_text_only() {
        let (mut b, mut rx) = backend();
        b.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_DIB)]);
        assert!(rx.try_recv().is_err());
        b.on_remote_copy(&[
            ClipboardFormat::new(ClipboardFormatId::CF_DIB),
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]);
        assert!(matches!(
            message(&mut rx),
            ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT)
        ));
    }
}
