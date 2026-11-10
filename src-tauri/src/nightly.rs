//! The newest Handrail Nightly build, for Settings › About. Nightly builds are prereleases of
//! this repository tagged `nightly-<version>` (see `.github/workflows/release.yml`); they install
//! as a separate app ("Handrail Nightly", its own data folder), so the stable app only offers the
//! installer for this system and opens it in the browser. It never installs it itself.
//!
//! The newest build is found through the nightly update channel: the `nightly-latest` prerelease
//! holds the latest.json of the newest nightly, whose version names its release. (The release
//! list is not ordered by date, and stable releases soon push nightlies off its first page.)

use aht_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// The update channel of nightly installs: the newest nightly's updater manifest.
const CHANNEL_MANIFEST: &str =
    "https://github.com/andreafunicelli/Handrail/releases/download/nightly-latest/latest.json";
const RELEASE_BY_TAG_API: &str =
    "https://api.github.com/repos/andreafunicelli/Handrail/releases/tags/";
/// Downloads offered must come from this repository's releases.
const DOWNLOAD_PREFIX: &str = "https://github.com/andreafunicelli/Handrail/releases/download/";
const PAGE_PREFIX: &str = "https://github.com/andreafunicelli/Handrail/releases/tag/";
const TAG_PREFIX: &str = "nightly-";

#[derive(Deserialize)]
struct Manifest {
    version: String,
}

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
    /// The installer for this system; `None` when the build has none (a system it is not built
    /// for).
    pub download: Option<NightlyAsset>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct NightlyAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A nightly version as the release workflow writes it (`1.1.0-212`); anything else is refused
/// before it goes into a URL.
fn valid_version(v: &str) -> bool {
    let Some((core, run)) = v.split_once('-') else {
        return false;
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .chain(std::iter::once(&run))
            .all(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit()))
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

/// The build described by the release of tag `nightly-<version>`, with its installer for `os`.
/// `None` unless it is that published nightly prerelease of this repository.
fn build_from(r: Release, version: &str, os: &str) -> Option<NightlyBuild> {
    let tag = format!("{TAG_PREFIX}{version}");
    if r.tag_name != tag || !r.prerelease || r.draft || r.html_url != format!("{PAGE_PREFIX}{tag}")
    {
        return None;
    }
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
        version: version.to_string(),
        page_url: r.html_url,
        published_at: r.published_at,
        download,
    })
}

/// GET `url` as JSON; `None` on 404.
async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<Option<T>> {
    let res = client
        .get(url)
        .header("User-Agent", "Handrail")
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let res = res
        .error_for_status()
        .map_err(|e| Error::Network(e.to_string()))?;
    res.json()
        .await
        .map(Some)
        .map_err(|e| Error::Network(e.to_string()))
}

/// The newest Handrail Nightly build, or `None` when none is published.
#[tauri::command(rename_all = "snake_case")]
pub async fn nightly_build() -> Result<Option<NightlyBuild>> {
    let client = reqwest::Client::new();
    let Some(manifest) = get_json::<Manifest>(&client, CHANNEL_MANIFEST).await? else {
        return Ok(None);
    };
    if !valid_version(&manifest.version) {
        return Err(Error::Invalid(format!(
            "unexpected nightly version {:?}",
            manifest.version
        )));
    }
    let url = format!("{RELEASE_BY_TAG_API}{TAG_PREFIX}{}", manifest.version);
    let Some(release) = get_json::<Release>(&client, &url).await? else {
        return Ok(None);
    };
    Ok(build_from(release, &manifest.version, std::env::consts::OS))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "1.1.0-212";

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            browser_download_url: format!("{DOWNLOAD_PREFIX}nightly-{V}/{name}"),
            size: 7,
        }
    }

    fn release(assets: Vec<Asset>) -> Release {
        Release {
            tag_name: format!("nightly-{V}"),
            prerelease: true,
            draft: false,
            html_url: format!("{PAGE_PREFIX}nightly-{V}"),
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
    fn versions_from_the_channel() {
        assert!(valid_version("1.1.0-212"));
        assert!(valid_version("10.20.30-65535"));
        for bad in [
            "1.1.0",
            "latest",
            "1.1-2",
            "1.1.0-",
            "1.1.0-2/../x",
            "1.1.0-a",
            "-1",
            "1..0-2",
            "",
        ] {
            assert!(!valid_version(bad), "{bad}");
        }
    }

    #[test]
    fn installer_for_each_system() {
        let mac = build_from(release(all_assets()), V, "macos").unwrap();
        assert_eq!(mac.version, V);
        assert_eq!(mac.page_url, format!("{PAGE_PREFIX}nightly-{V}"));
        let dl = mac.download.unwrap();
        assert_eq!(dl.name, "Handrail.Nightly_1.1.0-212_universal.dmg");
        assert_eq!(dl.size, 7);
        let win = build_from(release(all_assets()), V, "windows").unwrap();
        assert!(win.download.unwrap().name.ends_with("-setup.exe"));
        let linux = build_from(release(all_assets()), V, "linux").unwrap();
        assert!(linux.download.unwrap().name.ends_with(".AppImage"));
        assert_eq!(
            build_from(release(all_assets()), V, "freebsd")
                .unwrap()
                .download,
            None
        );
    }

    #[test]
    fn falls_back_to_the_msi_and_skips_foreign_links() {
        let mut foreign = asset("evil-setup.exe");
        foreign.browser_download_url = "https://example.com/evil-setup.exe".into();
        let b = build_from(
            release(vec![
                asset("Handrail.Nightly_1.1.0-212_x64_en-US.msi"),
                foreign,
            ]),
            V,
            "windows",
        );
        assert!(b.unwrap().download.unwrap().name.ends_with(".msi"));
    }

    #[test]
    fn only_that_published_nightly_of_this_repository() {
        let mut r = release(all_assets());
        r.html_url = "https://example.com/x".into();
        assert_eq!(build_from(r, V, "macos"), None);
        let mut r = release(all_assets());
        r.draft = true;
        assert_eq!(build_from(r, V, "macos"), None);
        let mut r = release(all_assets());
        r.prerelease = false;
        assert_eq!(build_from(r, V, "macos"), None);
        assert_eq!(
            build_from(release(all_assets()), "1.1.0-213", "macos"),
            None
        );
    }
}
