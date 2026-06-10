//! Handles checking for application updates via the GitHub Release API.
//!
//! This module queries the official GitHub repository for the latest release,
//! compares the version tag against the currently compiled version, and extracts
//! the download URL for the `.msi` installer.

use anyhow::Result;
use serde::Deserialize;
use std::env;

const REPO_URL: &str = "https://api.github.com/repos/congchuahiep/WinGlide/releases/latest";

/// Represents the JSON structure of a GitHub Release.
#[derive(Deserialize, Debug)]
pub struct Release {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<Asset>,
}

/// Represents a downloadable asset attached to a GitHub Release.
#[derive(Deserialize, Debug)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// Contains the details of an available update.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub download_url: String,
    pub release_notes: Option<String>,
}

/// Checks the GitHub repository for a newer version of the application.
///
/// This function performs a synchronous HTTP GET request to the GitHub API.
/// If a newer version is found, it attempts to locate the `.msi` asset from the release
/// and returns the `UpdateInfo`.
///
/// # Returns
///
/// - `Ok(Some(UpdateInfo))` if a new update is available.
/// - `Ok(None)` if the application is already up-to-date.
/// - `Err` if the network request fails or the JSON response is invalid.
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

    // Extract the tag name (stripping the 'v' prefix if it exists)
    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    // Simple string comparison for versions
    if latest_version != current_version {
        // Look for the MSI installer in the release assets
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
