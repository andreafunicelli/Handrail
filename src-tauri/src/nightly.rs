//! The newest Handrail Nightly build, for Settings › About. Nightly builds are prereleases of
//! this repository tagged `nightly-<version>` (see `.github/workflows/release.yml`); they install
//! as a separate app ("Handrail Nightly", its own data folder), so the stable app only offers the
//! installer for this system and opens it in the browser. It never installs it itself.

use aht_core::{Error, Result};
use serde::{Deserialize, Serialize};

const RELEASES_API: &str =
    "https://api.github.com/repos/andreafunicelli/Handrail/releases?per_page=30";
/// Downloads offered must come from this repository's releases.
const DOWNLOAD_PREFIX: &str = "https://github.com/andreafunicelli/Handrail/releases/download/";
const PAGE_PREFIX: &str = "https://github.com/andreafunicelli/Handrail/releases/tag/";
const TAG_PREFIX: &str = "nightly-";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct NightlyBuild {
    /// `1.1.0-212`
    pub version: String,
    /// The release page (notes and every installer).
    pub page_url: String,
    pub published_at: Option<String>,
    /// The installer for this system; `None` when the build has none (still building, or a
    /// system it is not built for).
    pub download: Option<NightlyAsset>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct NightlyAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// The installer file suffixes for an OS, best first.
fn installer_suffixes(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &[".dmg"],
        // The NSIS installer is what the updater uses too; the .msi is the fallback.
        "windows" => &["-setup.exe", ".msi"],
        // The AppImage installs beside a stable .deb/.rpm; the packages would conflict.
        "linux" => &[".AppImage"],
        _ => &[],
    }
}

/// The newest published nightly in `releases` (the API lists newest first) and its installer
/// for `os`. Releases whose links do not point at this repository are skipped.
fn pick(releases: Vec<Release>, os: &str) -> Option<NightlyBuild> {
    let r = releases.into_iter().find(|r| {
        r.prerelease
            && !r.draft
            // `nightly-<version>`, not the `nightly-latest` update channel.
            && r.tag_name
                .strip_prefix(TAG_PREFIX)
                .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
            && r.html_url.starts_with(PAGE_PREFIX)
    })?;
    let download = installer_suffixes(os).iter().find_map(|suffix| {
        r.assets
            .iter()
            .find(|a| {
                a.name.ends_with(suffix) && a.browser_download_url.starts_with(DOWNLOAD_PREFIX)
            })
            .map(|a| NightlyAsset {
                name: a.name.clone(),
                url: a.browser_download_url.clone(),
                size: a.size,
            })
    });
    Some(NightlyBuild {
        version: r.tag_name[TAG_PREFIX.len()..].to_string(),
        page_url: r.html_url,
        published_at: r.published_at,
        download,
    })
}

/// The newest Handrail Nightly build, or `None` when none is published.
#[tauri::command(rename_all = "snake_case")]
pub async fn nightly_build() -> Result<Option<NightlyBuild>> {
    let releases: Vec<Release> = reqwest::Client::new()
        .get(RELEASES_API)
        .header("User-Agent", "Handrail")
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?
        .error_for_status()
        .map_err(|e| Error::Network(e.to_string()))?
        .json()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    Ok(pick(releases, std::env::consts::OS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            browser_download_url: format!("{DOWNLOAD_PREFIX}nightly-1.1.0-212/{name}"),
            size: 7,
        }
    }

    fn release(tag: &str, prerelease: bool, assets: Vec<Asset>) -> Release {
        Release {
            tag_name: tag.into(),
            prerelease,
            draft: false,
            html_url: format!("{PAGE_PREFIX}{tag}"),
            published_at: Some("2026-11-10T22:00:00Z".into()),
            assets,
        }
    }

    fn all_assets() -> Vec<Asset> {
        [
            "Handrail.Nightly_1.1.0-212_universal.dmg",
            "Handrail.Nightly_1.1.0-212_x64-setup.exe",
            "Handrail.Nightly_1.1.0-212_x64_en-US.msi",
            "Handrail.Nightly_1.1.0-212_amd64.AppImage",
            "Handrail.Nightly_1.1.0-212_amd64.deb",
            "latest.json",
        ]
        .into_iter()
        .map(asset)
        .collect()
    }

    #[test]
    fn picks_the_newest_nightly_and_the_installer_for_each_system() {
        let list = || {
            vec![
                release(
                    "v1.0.50",
                    false,
                    vec![asset("Handrail_1.0.50_universal.dmg")],
                ),
                release("nightly-latest", true, vec![asset("latest.json")]),
                release("nightly-1.1.0-212", true, all_assets()),
                release("nightly-1.1.0-200", true, all_assets()),
            ]
        };
        // `nightly-latest` (the update channel, only a latest.json) must not hide the real build.
        let b = pick(list(), "macos").unwrap();
        assert_eq!(b.version, "1.1.0-212");
        assert_eq!(b.page_url, format!("{PAGE_PREFIX}nightly-1.1.0-212"));
        let dl = b.download.unwrap();
        assert_eq!(dl.name, "Handrail.Nightly_1.1.0-212_universal.dmg");
        assert_eq!(dl.size, 7);
    }

    #[test]
    fn installers_per_system() {
        let list = || {
            vec![
                release("v1.0.50", false, vec![]),
                release("nightly-1.1.0-212", true, all_assets()),
            ]
        };
        let mac = pick(list(), "macos").unwrap();
        assert_eq!(mac.version, "1.1.0-212");
        assert!(mac.download.unwrap().name.ends_with(".dmg"));
        assert!(pick(list(), "windows")
            .unwrap()
            .download
            .unwrap()
            .name
            .ends_with("-setup.exe"));
        assert!(pick(list(), "linux")
            .unwrap()
            .download
            .unwrap()
            .name
            .ends_with(".AppImage"));
        assert_eq!(pick(list(), "freebsd").unwrap().download, None);
    }

    #[test]
    fn falls_back_to_the_msi_and_skips_foreign_links() {
        let mut msi_only = vec![asset("Handrail.Nightly_1.1.0-212_x64_en-US.msi")];
        let mut foreign = asset("evil-setup.exe");
        foreign.browser_download_url = "https://example.com/evil-setup.exe".into();
        msi_only.push(foreign);
        let b = pick(
            vec![release("nightly-1.1.0-212", true, msi_only)],
            "windows",
        )
        .unwrap();
        assert!(b.download.unwrap().name.ends_with(".msi"));

        let mut r = release("nightly-1.1.0-213", true, all_assets());
        r.html_url = "https://example.com/x".into();
        assert_eq!(pick(vec![r], "macos"), None);
    }

    #[test]
    fn no_nightly() {
        assert_eq!(pick(vec![release("v1.0.50", false, vec![])], "macos"), None);
        let mut draft = release("nightly-1.1.0-212", true, all_assets());
        draft.draft = true;
        assert_eq!(pick(vec![draft], "macos"), None);
    }
}
