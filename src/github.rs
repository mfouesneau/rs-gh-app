use anyhow::{Result, anyhow};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use std::{env, fmt};

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub label: Option<String>,
    pub content_type: Option<String>,
    pub size: u64,
    pub download_count: u64,
    pub browser_download_url: Option<String>,
    // other fields are available
    // but not super useful for general use
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (id: {}, size: {} bytes, downloads: {}, url: {})",
            self.name,
            self.id,
            self.size,
            self.download_count,
            self.browser_download_url
                .as_deref()
                .unwrap_or("<no browser url>")
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    pub assets: Vec<Asset>,
    // other fields are available
    // but not super useful for general use
}

/// Check the GitHub API rate limit and print the remaining limit and reset time.
pub async fn check_rate_limit() -> Result<()> {
    let client = reqwest::Client::new();

    // Check rate limit first
    let rate_limit_response = client
        .get("https://api.github.com/rate_limit")
        .header("User-Agent", "gh-app-installer/0.1.0")
        .send()
        .await?;

    if !rate_limit_response.status().is_success() {
        println!("⚠️  Could not check rate limit, proceeding anyway");
        Ok(())
    } else {
        let rate_limit_text = rate_limit_response.text().await?;
        match serde_json::from_str::<serde_json::Value>(&rate_limit_text) {
            Ok(rate_limit) => {
                let remaining = rate_limit["rate"]["remaining"].as_u64().unwrap_or(1);
                let reset_time = rate_limit["rate"]["reset"].as_u64().unwrap_or(0);
                let reset_datetime =
                    chrono::DateTime::from_timestamp(reset_time as i64, 0).unwrap_or_default();
                let now = chrono::Utc::now();
                let time_until_reset = reset_datetime.signed_duration_since(now);

                let delta_str = if time_until_reset.num_seconds() <= 0 {
                    "should reset now".to_string()
                } else if time_until_reset.num_hours() > 0 {
                    format!("in {}hrs", time_until_reset.num_hours())
                } else if time_until_reset.num_minutes() > 0 {
                    format!("in {}min", time_until_reset.num_minutes())
                } else {
                    "very soon".to_string()
                };

                if remaining > 0 {
                    println!("✅  Rate limit remaining: {}", remaining);
                    println!(
                        "ℹ️  Rate limit reset at: {} {}",
                        reset_datetime.format("%Y-%m-%d %H:%M:%S UTC"),
                        delta_str
                    );
                    return Ok(());
                }

                return Err(anyhow::anyhow!(
                    "🚨 GitHub API rate limit exceeded. Resets at: {} ({})",
                    reset_datetime.format("%Y-%m-%d %H:%M:%S UTC"),
                    delta_str
                ));
            }
            _ => Err(anyhow::anyhow!("Unexpected response from GitHub API")),
        }
    }
}

/// Fetch the assets of the latest GitHub Release for a repository given as "owner/repo".
///
/// - `repo` must be in the form "owner/repo".
/// - `token` is an optional GitHub token (useful for private repos and to raise rate limits).
///
/// Returns a Release or an Error if the repository has no Release (GitHub returns 404 for "no release").
pub async fn fetch_latest_release(repo: &str, token: Option<&str>) -> Result<Release> {
    // check repo format
    let mut parts = repo.splitn(2, '/');
    let owner = parts.next().ok_or_else(|| anyhow!("invalid repo format"))?;
    let name = parts.next().ok_or_else(|| anyhow!("invalid repo format"))?;

    //build url
    let url = format!(
        "https://api.github.com/repos/{owner}/{name}/releases/latest",
        owner = owner,
        name = name
    );

    // set the client
    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header(USER_AGENT, "gh_release_assets")
        .header(ACCEPT, "application/vnd.github+json");
    if let Some(t) = token {
        req = req.header(AUTHORIZATION, format!("Bearer {}", t));
    }

    // send request
    let resp = req.send().await?;

    // check response
    match resp.status() {
        reqwest::StatusCode::OK => {
            let release: Release = resp.json().await?;
            Ok(release)
        }
        reqwest::StatusCode::NOT_FOUND => {
            // No release for that repo (or repo not found). Choose how you want to handle this.
            // Here we return an empty list (caller can distinguish with additional checks if needed).
            Err(anyhow!("No release found"))
        }
        s => {
            let text = resp.text().await.unwrap_or_default();
            Err(anyhow!(
                "GitHub API returned error {}: {}",
                s.as_u16(),
                text
            ))
        }
    }
}

impl Release {
    pub async fn fetch_latest(repo: &str, token: Option<&str>) -> Self {
        let release = fetch_latest_release(repo, token).await;
        if release.is_ok() {
            let release = release.unwrap();
            Self {
                tag_name: release.tag_name,
                html_url: release.html_url,
                assets: release.assets,
            }
        } else {
            Self {
                tag_name: String::new(),
                html_url: String::new(),
                assets: Vec::new(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

impl Platform {
    pub fn current() -> Self {
        Self {
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
        }
    }

    pub fn to_string(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

pub struct PlatformMatcher {
    pub arch_aliases: std::collections::HashMap<String, Vec<String>>,
    pub os_aliases: std::collections::HashMap<String, Vec<String>>,
}

impl Default for PlatformMatcher {
    fn default() -> Self {
        let mut arch_aliases = std::collections::HashMap::new();
        arch_aliases.insert(
            "aarch64".to_string(),
            vec!["arm64".to_string(), "aarch64".to_string()],
        );
        arch_aliases.insert(
            "x86_64".to_string(),
            vec!["x86_64".to_string(), "amd64".to_string()],
        );
        arch_aliases.insert(
            "arm".to_string(),
            vec!["arm".to_string(), "armv6".to_string(), "armv7".to_string()],
        );

        let mut os_aliases = std::collections::HashMap::new();
        os_aliases.insert(
            "macos".to_string(),
            vec!["macos".to_string(), "darwin".to_string(), "osx".to_string()],
        );
        os_aliases.insert("linux".to_string(), vec!["linux".to_string()]);
        os_aliases.insert(
            "windows".to_string(),
            vec![
                "windows".to_string(),
                "win32".to_string(),
                "win".to_string(),
            ],
        );

        Self {
            arch_aliases,
            os_aliases,
        }
    }
}

pub fn asset_matcher(
    asset_name: &str,
    matcher: Option<&PlatformMatcher>,
    current_platform: Option<&Platform>,
) -> Result<()> {
    let matcher = match matcher {
        Some(m) => m,
        None => &PlatformMatcher::default(),
    };
    let current_platform = match current_platform {
        Some(p) => p,
        None => &Platform::current(),
    };

    // Try to find exact matches first
    let os_aliases = matcher.os_aliases.get(&current_platform.os);
    let arch_aliases = matcher.arch_aliases.get(&current_platform.arch);

    let name = asset_name.to_lowercase();
    // direct match
    if name.contains(&current_platform.os) && name.contains(&current_platform.arch) {
        return Ok(());
    }
    // Try to find matches using aliases
    if let (Some(os_aliases), Some(arch_aliases)) = (os_aliases, arch_aliases) {
        for os_alias in os_aliases {
            for arch_alias in arch_aliases {
                if name.contains(os_alias) && name.contains(arch_alias) {
                    return Ok(());
                }
            }
        }
    } else if let Some(os_aliases) = os_aliases {
        for os_alias in os_aliases {
            if name.contains(os_alias) {
                return Ok(());
            }
        }
    } else if let Some(arch_aliases) = arch_aliases {
        for arch_alias in arch_aliases {
            if name.contains(arch_alias) {
                return Ok(());
            }
        }
    }
    Err(anyhow::anyhow!("No match found"))
}

pub fn find_platform_assets<'a>(
    assets: &'a Vec<Asset>,
    matcher: Option<&PlatformMatcher>,
    current_platform: Option<&Platform>,
) -> Result<Vec<&'a Asset>> {
    // provide default matcher and platform if not provided
    let matcher = match matcher {
        Some(m) => m,
        None => &PlatformMatcher::default(),
    };
    let current_platform = match current_platform {
        Some(p) => p,
        None => &Platform::current(),
    };

    let matched_assets = assets
        .iter()
        .filter(|asset| asset_matcher(&asset.name, Some(&matcher), Some(&current_platform)).is_ok())
        .collect::<Vec<_>>();

    Ok(matched_assets)
}
