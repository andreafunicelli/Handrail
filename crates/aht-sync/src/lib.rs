//! aht-sync — sync of the end-to-end-encrypted vault through a private GitHub repo or a local
//! folder (iCloud/Dropbox/Syncthing). See docs/superpowers/specs/2026-10-10-aihubterm-design.md.
//!
//! Remote compaction of tombstones (deleting remote item files) is out of scope for v1: remote
//! files are never deleted by [`sync_once`].

mod backend;
mod engine;
mod error;
mod folder;
pub mod github;
pub mod github_device_flow;

pub use backend::{Change, RemoteEntry, SyncBackend};
pub use engine::{fetch_manifest, sync_once, SyncReport, SyncState};
pub use error::{Result, SyncError};
pub use folder::FolderBackend;
pub use github::GitHubBackend;
