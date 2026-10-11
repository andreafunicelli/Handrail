//! aht-term — see docs/superpowers/specs/2026-10-10-aihubterm-design.md
pub mod hooks;
pub mod parser;
pub mod policy;
pub mod session;
pub mod store;

pub use policy::{classify, Policy, PolicyVerdict};
pub use session::{Approver, SessionEvent, TermConfig, TermSession};
pub use store::OutputStore;
