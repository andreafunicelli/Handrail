//! Shared types for Handrail. Every other crate depends on this one; keep it free of I/O.

pub mod agent;
pub mod error;
pub mod model;
pub mod terminal;

pub use error::{Error, Result};

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

id_type!(ConnectionId);
id_type!(GroupId);
id_type!(SecretId);
id_type!(ProviderId);
id_type!(HarnessProfileId);
id_type!(SnippetId);
id_type!(S3AccountId);
id_type!(IdentityId);
id_type!(SessionId);
id_type!(OutputId);
id_type!(ApprovalId);
id_type!(DeviceId);
