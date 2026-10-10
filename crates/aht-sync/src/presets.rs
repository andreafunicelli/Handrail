//! Cloud-drive folders a user can pick for folder sync with one click.
//!
//! Only folders that exist are offered: the desktop client of that service must already be
//! installed and signed in. The vault goes into a `Handrail` subfolder (or the `AIHubTerm` one
//! that builds released under the old name created, if it holds a vault).

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Subfolder created inside the cloud drive.
pub const FOLDER_NAME: &str = "Handrail";
/// Subfolder used by builds released as "AIHubTerm"; reused when it already holds a vault.
pub const LEGACY_FOLDER_NAME: &str = "AIHubTerm";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CloudDrive {
    #[serde(rename = "icloud")]
    ICloud,
    #[serde(rename = "dropbox")]
    Dropbox,
    #[serde(rename = "onedrive")]
    OneDrive,
    #[serde(rename = "google_drive")]
    GoogleDrive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FolderPreset {
    pub drive: CloudDrive,
    /// e.g. "iCloud Drive", "OneDrive - Contoso", "Google Drive (me@example.com)".
    pub label: String,
    /// Where the vault goes: `<drive root>/Handrail`.
    pub path: PathBuf,
    /// The folder already holds a synced vault (another device set it up): join it.
    pub has_vault: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// Subdirectories of `dir` whose names start with `prefix`, sorted by name.
fn dirs_with_prefix(dir: &Path, prefix: &str) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(prefix).then(|| (name, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Detected cloud drives under `home`, at most one per service.
pub fn folder_presets(home: &Path, os: Os) -> Vec<FolderPreset> {
    let mut found: Vec<(CloudDrive, String, PathBuf)> = Vec::new();
    let mut add = |drive: CloudDrive, label: String, root: PathBuf| {
        if root.is_dir() && !found.iter().any(|(d, _, _)| *d == drive) {
            found.push((drive, label, root));
        }
    };
    match os {
        Os::MacOs => {
            let cloud = home.join("Library").join("CloudStorage");
            add(
                CloudDrive::ICloud,
                "iCloud Drive".into(),
                home.join("Library")
                    .join("Mobile Documents")
                    .join("com~apple~CloudDocs"),
            );
            // Dropbox moved to File Provider (~/Library/CloudStorage) in 2023; older installs
            // still use ~/Dropbox.
            add(CloudDrive::Dropbox, "Dropbox".into(), cloud.join("Dropbox"));
            add(CloudDrive::Dropbox, "Dropbox".into(), home.join("Dropbox"));
            // One folder per account: "OneDrive-Personal", "OneDrive-Contoso".
            for (name, p) in dirs_with_prefix(&cloud, "OneDrive") {
                let label = match name.strip_prefix("OneDrive-") {
                    Some("Personal") | None => "OneDrive".to_string(),
                    Some(org) => format!("OneDrive - {org}"),
                };
                add(CloudDrive::OneDrive, label, p);
            }
            // "GoogleDrive-me@example.com/My Drive".
            for (name, p) in dirs_with_prefix(&cloud, "GoogleDrive-") {
                let account = name.trim_start_matches("GoogleDrive-");
                add(
                    CloudDrive::GoogleDrive,
                    format!("Google Drive ({account})"),
                    p.join("My Drive"),
                );
            }
        }
        Os::Windows => {
            add(
                CloudDrive::ICloud,
                "iCloud Drive".into(),
                home.join("iCloudDrive"),
            );
            add(CloudDrive::Dropbox, "Dropbox".into(), home.join("Dropbox"));
            // "OneDrive" (personal) or "OneDrive - Contoso" (work).
            for (name, p) in dirs_with_prefix(home, "OneDrive") {
                add(CloudDrive::OneDrive, name, p);
            }
            // Google Drive for desktop mounts a drive letter, not a folder in the profile.
        }
        Os::Other => {
            add(CloudDrive::Dropbox, "Dropbox".into(), home.join("Dropbox"));
        }
    }
    found
        .into_iter()
        .map(|(drive, label, root)| {
            let has_vault = |p: &Path| p.join("manifest.json").is_file();
            let current = root.join(FOLDER_NAME);
            let legacy = root.join(LEGACY_FOLDER_NAME);
            let path = if !has_vault(&current) && has_vault(&legacy) {
                legacy
            } else {
                current
            };
            FolderPreset {
                drive,
                label,
                has_vault: has_vault(&path),
                path,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkdir(p: PathBuf) -> PathBuf {
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn drive_names_on_the_wire() {
        let names: Vec<serde_json::Value> = [
            CloudDrive::ICloud,
            CloudDrive::Dropbox,
            CloudDrive::OneDrive,
            CloudDrive::GoogleDrive,
        ]
        .iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .collect();
        assert_eq!(
            names,
            ["icloud", "dropbox", "onedrive", "google_drive"].map(serde_json::Value::from)
        );
    }

    #[test]
    fn nothing_installed_offers_nothing() {
        let home = tempfile::tempdir().unwrap();
        for os in [Os::MacOs, Os::Windows, Os::Other] {
            assert!(folder_presets(home.path(), os).is_empty(), "{os:?}");
        }
    }

    #[test]
    fn macos_cloud_drives() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let cloud = h.join("Library").join("CloudStorage");
        let icloud = mkdir(
            h.join("Library")
                .join("Mobile Documents")
                .join("com~apple~CloudDocs"),
        );
        mkdir(cloud.join("Dropbox"));
        mkdir(h.join("Dropbox")); // legacy location: the File Provider one wins
        mkdir(cloud.join("OneDrive-Contoso"));
        mkdir(cloud.join("OneDrive-Personal"));
        mkdir(cloud.join("GoogleDrive-me@example.com").join("My Drive"));
        // An existing vault in iCloud: offered as "join".
        mkdir(icloud.join(FOLDER_NAME));
        std::fs::write(icloud.join(FOLDER_NAME).join("manifest.json"), b"{}").unwrap();

        let p = folder_presets(h, Os::MacOs);
        let got: Vec<(CloudDrive, &str, bool)> = p
            .iter()
            .map(|x| (x.drive, x.label.as_str(), x.has_vault))
            .collect();
        assert_eq!(
            got,
            [
                (CloudDrive::ICloud, "iCloud Drive", true),
                (CloudDrive::Dropbox, "Dropbox", false),
                (CloudDrive::OneDrive, "OneDrive - Contoso", false),
                (
                    CloudDrive::GoogleDrive,
                    "Google Drive (me@example.com)",
                    false
                ),
            ]
        );
        assert_eq!(p[0].path, icloud.join(FOLDER_NAME));
        assert_eq!(p[1].path, cloud.join("Dropbox").join(FOLDER_NAME));
        assert_eq!(
            p[3].path,
            cloud
                .join("GoogleDrive-me@example.com")
                .join("My Drive")
                .join(FOLDER_NAME)
        );
    }

    #[test]
    fn macos_legacy_dropbox_and_personal_onedrive() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        mkdir(h.join("Dropbox"));
        mkdir(
            h.join("Library")
                .join("CloudStorage")
                .join("OneDrive-Personal"),
        );
        let p = folder_presets(h, Os::MacOs);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].path, h.join("Dropbox").join(FOLDER_NAME));
        assert_eq!(p[1].label, "OneDrive");
    }

    #[test]
    fn a_vault_in_the_legacy_folder_is_reused() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let dropbox = mkdir(h.join("Dropbox"));
        mkdir(dropbox.join(LEGACY_FOLDER_NAME));
        std::fs::write(
            dropbox.join(LEGACY_FOLDER_NAME).join("manifest.json"),
            b"{}",
        )
        .unwrap();
        let p = folder_presets(h, Os::Other);
        assert_eq!(p[0].path, dropbox.join(LEGACY_FOLDER_NAME));
        assert!(p[0].has_vault);
        // Once the new folder holds a vault, it wins.
        mkdir(dropbox.join(FOLDER_NAME));
        std::fs::write(dropbox.join(FOLDER_NAME).join("manifest.json"), b"{}").unwrap();
        assert_eq!(
            folder_presets(h, Os::Other)[0].path,
            dropbox.join(FOLDER_NAME)
        );
    }

    #[test]
    fn windows_cloud_drives() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        mkdir(h.join("iCloudDrive"));
        mkdir(h.join("OneDrive - Contoso"));
        // A file, not a folder: ignored.
        std::fs::write(h.join("Dropbox"), b"").unwrap();
        let p = folder_presets(h, Os::Windows);
        let got: Vec<(CloudDrive, &str)> = p.iter().map(|x| (x.drive, x.label.as_str())).collect();
        assert_eq!(
            got,
            [
                (CloudDrive::ICloud, "iCloud Drive"),
                (CloudDrive::OneDrive, "OneDrive - Contoso"),
            ]
        );
        assert_eq!(p[0].path, h.join("iCloudDrive").join(FOLDER_NAME));
    }
}
