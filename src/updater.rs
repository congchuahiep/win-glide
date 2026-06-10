use anyhow::Result;
use serde::Deserialize;
use std::env;

const REPO_URL: &str = "https://api.github.com/repos/congchuahiep/WinGlide/releases/latest";

#[derive(Deserialize, Debug)]
pub struct Release {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize, Debug)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub download_url: String,
    pub release_notes: Option<String>,
}

pub fn check_for_updates() -> Result<Option<UpdateInfo>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("WinGlide-Updater")
        .build()?;

    let response = client.get(REPO_URL).send()?;
    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch release info: {}", response.status());
    }

    let release: Release = response.json()?;
    let current_version = env!("CARGO_PKG_VERSION");

    // Lấy tag_name (loại bỏ 'v' ở đầu nếu có)
    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    // So sánh version đơn giản
    if latest_version != current_version {
        // Tìm file MSI trong assets
        if let Some(asset) = release.assets.iter().find(|a| a.name.ends_with(".msi")) {
            return Ok(Some(UpdateInfo {
                latest_version,
                download_url: asset.browser_download_url.clone(),
                release_notes: release.body.clone(),
            }));
        }
    }

    Ok(None)
}
