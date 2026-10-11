//! Typed items stored in the vault.

use aht_core::model::{Connection, Group, HarnessProfile, Provider, S3Account, Secret, Snippet};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

/// A record type that can be stored in the vault. `TYPE` must be unique per type and stable
/// forever (it is part of the encrypted envelope and of the file-id derivation).
pub trait VaultItem: Serialize + DeserializeOwned {
    const TYPE: &'static str;
    fn id(&self) -> String;
}

/// A generic key/value setting (approval settings, snippets, preferences...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: Value,
}

impl VaultItem for Setting {
    const TYPE: &'static str = "setting";
    fn id(&self) -> String {
        self.key.clone()
    }
}

macro_rules! impl_item {
    ($t:ty, $name:literal) => {
        impl VaultItem for $t {
            const TYPE: &'static str = $name;
            fn id(&self) -> String {
                self.id.to_string()
            }
        }
    };
}

impl_item!(Connection, "connection");
impl_item!(Group, "group");
impl_item!(Secret, "secret");
impl_item!(Provider, "provider");
impl_item!(HarnessProfile, "harness_profile");
impl_item!(Snippet, "snippet");
impl_item!(S3Account, "s3_account");
