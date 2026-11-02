use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use tokio::sync::mpsc;
use tracing::error;

use crate::rdp::RdpInputEvent;

/// Shim that forwards CLIPRDR events into the `RdpInputEvent` channel.
#[derive(Clone, Debug)]
pub(crate) struct ClientClipboardMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl ClientClipboardMessageProxy {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }
}

impl ClipboardMessageProxy for ClientClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.tx.send(RdpInputEvent::Clipboard(message)).is_err() {
            error!("Failed to send clipboard message; receiver is closed");
        }
    }
}

/// Handrail patch: the Windows clipboard on a thread of its own, with a message loop.
///
/// Upstream creates `WinClipboard` on the thread that runs the session. That thread runs the
/// async runtime and never dispatches window messages, so the clipboard window saw no clipboard
/// changes and no backend events; and its `WM_RENDERFORMAT` handler blocks, waiting for data that
/// the same thread would have to receive. Here the window lives on its own thread, which
/// dispatches its messages until the session ends (dropping this posts `WM_QUIT`).
#[cfg(windows)]
pub(crate) struct WinClipboardThread {
    thread_id: u32,
}

#[cfg(windows)]
impl WinClipboardThread {
    pub(crate) fn spawn(
        proxy: ClientClipboardMessageProxy,
    ) -> std::io::Result<(Self, Box<dyn ironrdp_cliprdr::backend::CliprdrBackendFactory + Send>)> {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage};

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("cliprdr".to_owned()).spawn(move || {
            let clipboard = match ironrdp_cliprdr_native::WinClipboard::new(proxy) {
                Ok(clipboard) => clipboard,
                Err(e) => {
                    let _ = tx.send(Err(std::io::Error::other(e.to_string())));
                    return;
                }
            };
            // SAFETY: no preconditions.
            let thread_id = unsafe { GetCurrentThreadId() };
            if tx.send(Ok((thread_id, clipboard.backend_factory()))).is_err() {
                return;
            }
            let mut msg = MSG::default();
            // SAFETY: `msg` is a valid MSG to fill; 0 means WM_QUIT, -1 an error.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                // SAFETY: `msg` was just filled by GetMessageW.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            drop(clipboard);
        })?;
        let (thread_id, factory) = rx
            .recv()
            .map_err(|_| std::io::Error::other("the clipboard thread ended"))??;
        Ok((Self { thread_id }, factory))
    }
}

#[cfg(windows)]
impl Drop for WinClipboardThread {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        // SAFETY: posting to a thread id; fails harmlessly if the thread is gone.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}
