# Handrail patches to ironrdp-client 0.1.0

Source: IronRDP commit `11a0810c` (`chore(release): prepare for publishing (#1364)`),
`crates/ironrdp-client`, with the workspace-inherited fields of `Cargo.toml` written out and
the path dependencies replaced by their crates.io versions.

- The Windows clipboard runs on a thread of its own with a message loop (clipboard.rs,
  `WinClipboardThread`, used by `RdpClient::run` in rdp.rs). Upstream creates `WinClipboard`
  on the thread that runs the session, which never dispatches window messages: local copies
  never reached the server, and pasting a remote copy blocked that thread in `WM_RENDERFORMAT`
  waiting for data only it could receive. The thread needs the `windows` crate (0.62, already
  used by ironrdp-cliprdr-native). If the clipboard cannot be set up, the session connects
  without one (upstream fails the connection).
- `RdpClient::with_cliprdr_backend_factory` (rdp.rs), as in later IronRDP versions: the helper
  supplies its own text clipboard backend on macOS and Linux (rdp-helper/src/clip.rs), where
  IronRDP has no native one.

Drop this directory and its `[patch.crates-io]` entry once an IronRDP release on crates.io
lets the application supply the clipboard backend factory (upstream `main` has
`RdpClient::with_cliprdr_backend_factory`).
