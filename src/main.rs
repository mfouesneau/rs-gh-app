mod app;
mod github;
use anyhow::{Context, Result};
use app::{App, AppStatus, InstallationMethod, extract_version_from_string};
use clap::{Parser, Subcommand};
use github::{Release, check_rate_limit};
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::{env, fs};
use tempfile::TempDir;

// app.yaml format =================================================
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub apps: Vec<App>,
}

/// check for a configuration file in order or priority:
/// provided path, in the current directory, and the directory of the binary.
/// default configuration file name "apps.yaml"
///
/// # Arguments
///
/// * `config_file` - The path to the configuration file.
///
/// # Returns
///
/// A `Result` which is `Ok` if the configuration was loaded successfully,
/// or `Err` if there was an error.
fn locate_config_file(config_file: &str) -> Result<PathBuf> {
    let current_dir = env::current_dir()?;
    let binary_dir = env::current_exe()?.parent().unwrap().to_path_buf();

    let default_config_filename = "apps.yaml";
    let config_file_paths = vec![
        PathBuf::from(config_file),
        current_dir.join(default_config_filename),
        binary_dir.join(default_config_filename),
    ];

    for path in config_file_paths {
        if path.exists() {
            println!("ℹ️  Using configuration from {}", path.display());
            return Ok(path);
        }
    }

    Err(anyhow::anyhow!("No configuration file found"))
}

/// This function creates a sample configuration file with a few sample apps.
///
/// # Arguments
///
/// * `config_file` - The path to the configuration file.
///
/// # Returns
///
/// A `Result` which is `Ok` if the sample config file was created successfully,
/// or `Err` if there was an error.
async fn create_sample_config_file(config_file: &str) -> Result<()> {
    let sample_config = Config {
        apps: vec![
            App {
                name: "dust".to_string(),
                bin: "dust".to_string(),
                repo: Some("bootandy/dust".to_string()),
                description: Some("A disk usage analyzer".to_string()),
                install_command: None,
                update_command: None,
                version_command: None,
            },
            App {
                name: "bat".to_string(),
                bin: "bat".to_string(),
                description: Some("A cat clone with syntax highlighting".to_string()),
                repo: Some("sharkdp/bat".to_string()),
                install_command: None,
                update_command: None,
                version_command: None,
            },
            App {
                name: "uv".to_string(),
                bin: "uv".to_string(),
                repo: Some("astral-sh/uv".to_string()),
                install_command: Some("{download(https://astral.sh/uv/install.sh, /tmp/uv-install.sh)} && sh /tmp/uv-install.sh --bin-dir {bin_dir} --yes".to_string()),
                update_command: Some("{bin_path} self update".to_string()),
                description: Some("A fast python package manager".to_string()),
                version_command: None,
            }, ],
        };

    let yaml = serde_yaml::to_string(&sample_config)?;
    let config_sample_file = PathBuf::from(config_file);
    fs::write(&config_sample_file, yaml)?;
    println!(
        "📝 Created sample config file: {}",
        config_sample_file.display()
    );

    Ok(())
}

/// Load the configuration from the given file.
///
/// This function reads the configuration file and returns a `Config` struct.
/// If the file does not exist, it creates a sample configuration file.
///
/// # Arguments
///
/// * `config_file` - The path to the configuration file.
///
/// # Returns
///
/// A `Result` which is `Ok` if the configuration was loaded successfully,
/// or `Err` if there was an error.
async fn load_config(config_file: &str) -> Result<Config> {
    let config_path = locate_config_file(config_file);

    // if the config file does not exist, create a sample config file
    if config_path.is_err() {
        create_sample_config_file(config_file).await?;
    }
    let config_path = config_path.unwrap();

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    let config: Config =
        serde_yaml::from_str(&content).with_context(|| "Failed to parse YAML config")?;

    Ok(config)
}

/// Get the status and release information for the given application.
///
/// This function fetches the latest release information from GitHub for the given application.
/// It also checks the rate limit and retrieves the repository information.
///
/// # Arguments
///
/// * `app` - The application for which to fetch the status and release information.
///
/// # Returns
///
/// A `Result` containing a tuple with the application status and the latest release information.
async fn get_app_status_and_release(app: &App, debug: bool) -> Result<(AppStatus, Release)> {
    let mut status = AppStatus::new(app, debug);

    // check online assets and versions
    check_rate_limit(false).await?;

    let release_info: Release;
    let repo = status.app.get_repo();

    // get version from repo is any
    if repo.is_empty() {
        release_info = Release::default();
        // check if version_command is present
        if app.version_command.is_some() {
            let command = app.version_command.as_ref().unwrap();
            let processed_command = process_template(command, app, "").await?;
            println!(
                "   ⚙️ Getting latest version for {} with command\n\t {} ",
                app.name,
                processed_command.trim()
            );
            let output = Command::new("sh")
                .arg("-c")
                .arg(format!("{}", processed_command))
                .output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!(
                    "\n    Command: {}\n    Error:     {}",
                    processed_command,
                    stderr
                ));
            } else {
                // merge stdout into a string
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // parse stdout into a version
                if let Some(version) = extract_version_from_string(&stdout) {
                    println!("   ⚙️ Got {}", version.clone());
                    status.set_latest_version(version);
                } else {
                    println!("  ❓ Could not parse version from {}", stdout);
                }
            }
        }
    } else {
        release_info = Release::fetch_latest(repo, env::var("GITHUB_TOKEN").ok().as_deref()).await;
        if let Some(latest_version) = extract_version_from_string(&release_info.tag_name) {
            status.set_latest_version(latest_version);
        }
    }

    Ok((status, release_info))
}

/// Get the status and release information for the current application.
///
/// This function fetches the latest release information from GitHub for the current application.
/// It also checks the rate limit and retrieves the repository information.
///
/// This function differs from `get_app_status_and_release` in that the app and
/// status are internally set.
///
/// # Returns
///
/// A `Result` containing a tuple with the application status and the latest release information.
async fn get_thisapp_status_and_release() -> Result<(AppStatus, Release)> {
    let mut this_app_status = AppStatus {
        pixi_managed: Some(false),
        current_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        latest_version: None,
        app: App {
            name: "rs-gh-app".to_string(),
            repo: Some("mfouesneau/rs-gh-app".to_string()),
            bin: "rs-gh-app".to_string(),
            install_command: None,
            update_command: None,
            description: Some("A command-line tool for managing GitHub applications".to_string()),
            version_command: None,
        },
    };

    // check online assets and versions
    check_rate_limit(false).await?;
    let repo = this_app_status.app.get_repo();

    let release_info = Release::fetch_latest(repo, env::var("GITHUB_TOKEN").ok().as_deref()).await;

    if let Some(latest_version) = extract_version_from_string(&release_info.tag_name) {
        this_app_status.set_latest_version(latest_version);
    }

    Ok((this_app_status, release_info))
}
// Main CLI ================================================================
#[derive(Subcommand)]
enum Commands {
    /// Install or update applications
    Install {
        /// Application name to install (installs all if not specified)
        app_name: Option<String>,
        /// Preview what would be done without actually installing
        #[arg(long)]
        dry_run: bool,
    },
    /// Check versions without installing
    Check {
        /// Application name to check (checks all if not specified)
        app_name: Option<String>,
    },
    /// Update this tool to the latest version
    SelfUpdate {
        /// Preview what would be done without actually updating
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Parser)]
#[command(name = "rs-app-installer")]
#[command(about = "Install and update applications")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Stop on first error instead of continuing
    #[arg(long)]
    stop_on_error: bool,

    /// Path to the configuration file
    #[arg(short, long, default_value = "apps.yaml")]
    config: String,

    /// Debug mode
    #[arg(long)]
    debug: bool,
}

/// Get the directory where binaries are stored
///
/// Assumes `~/.local/bin` or provided by environment variable `bin_dir`.
///
/// Returns an error if the directory cannot be determined.
fn get_bin_dir() -> Result<PathBuf> {
    if let Ok(bin_dir) = std::env::var("bin_dir") {
        Ok(PathBuf::from(bin_dir))
    } else {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(".local").join("bin"))
    }
}

/// Extract a tar.gz archive to the specified destination path.
///
/// # Arguments
/// * `bytes` - The bytes of the tar.gz archive
/// * `dest_path` - The destination path to extract the archive to
///
/// # Returns
/// * `Ok(())` - If the extraction was successful
/// * `Err(Error)` - If the extraction failed
fn extract_tar_gz(bytes: &[u8], dest_path: &std::path::Path) -> Result<()> {
    let tar = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(tar);
    archive.unpack(dest_path)?;
    Ok(())
}

/// Extract a tar archive to the specified destination path.
///
/// # Arguments
/// * `bytes` - The bytes of the tar archive
/// * `dest_path` - The destination path to extract the archive to
///
/// # Returns
/// * `Ok(())` - If the extraction was successful
/// * `Err(Error)` - If the extraction failed
fn extract_tar(bytes: &[u8], dest_path: &std::path::Path) -> Result<()> {
    let mut archive = tar::Archive::new(bytes);
    archive.unpack(dest_path)?;
    Ok(())
}

/// Extract a zip archive to the specified destination path.
///
/// # Arguments
/// * `bytes` - The bytes of the zip archive
/// * `dest_path` - The destination path to extract the archive to
///
/// # Returns
/// * `Ok(())` - If the extraction was successful
/// * `Err(Error)` - If the extraction failed
fn extract_zip(bytes: &[u8], dest_path: &std::path::Path) -> Result<()> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = dest_path.join(file.name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(p)?;
                }
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))?;
            }
        }
    }
    Ok(())
}

/// Find a binary in an extracted directory
///
/// # Arguments
/// * `dir` - The directory to search in
/// * `bin_name` - The name of the binary to find
///
/// # Returns
/// * `Ok(PathBuf)` - The path to the binary if found
/// * `Err(Error)` - An error if the binary is not found
fn find_binary_in_extracted(dir: &std::path::Path, bin_name: &str) -> Result<PathBuf> {
    fn search_recursive(dir: &std::path::Path, bin_name: &str) -> Result<Option<PathBuf>> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(name) = path.file_name() {
                    if name.to_string_lossy() == bin_name
                        || name.to_string_lossy() == format!("{}.exe", bin_name)
                    {
                        return Ok(Some(path));
                    }
                }
            } else if path.is_dir() {
                if let Some(found) = search_recursive(&path, bin_name)? {
                    return Ok(Some(found));
                }
            }
        }
        Ok(None)
    }

    search_recursive(dir, bin_name)?
        .ok_or_else(|| anyhow::anyhow!("Binary '{}' not found in extracted archive", bin_name))
}

/// Filter the apps based on the given app name.
///
/// If `app_name` is `None`, all apps are returned.
/// If `app_name` is `Some(name)`, the app with the given name is returned.
///
/// # Arguments
/// * `apps` - The list of apps to filter.
/// * `app_name` - The name of the app to filter by.
///
/// # Returns
/// A `Result` containing a vector of `App` structs.
fn filter_apps(apps: &[App], app_name: Option<String>) -> Result<Vec<App>> {
    match app_name {
        Some(name) => {
            let app = apps
                .iter()
                .find(|app| app.name == name || app.bin == name)
                .ok_or_else(|| anyhow::anyhow!("App '{}' not found in configuration", name))?;
            Ok(vec![app.clone()])
        }
        None => Ok(apps.to_vec()),
    }
}

/// Get the best URL for the given release.
///
/// Returns the URL of the first asset that matches the current platform and has a valid download URL.
///
/// # Arguments
///
/// * `release` - The release to get the best URL for.
///
/// # Errors
///
/// Returns an error if no assets are found for the current platform or if there are multiple assets matching the current platform.
fn get_best_url(release: &Release) -> Result<String> {
    // get the first asset that matches with the platform with a valid download URL
    let matched_assets = github::find_platform_assets(&release.assets, None, None)?;
    let url: String;
    if matched_assets.is_empty() {
        return Err(anyhow::anyhow!(
            "❌ No assets found for the current platform"
        ));
    } else if matched_assets.len() > 1 {
        println!("⚠️  Multiple assets matching the current platform");
        matched_assets.iter().for_each(|asset| {
            println!("  - {}", asset);
        });
        let selected: Vec<_> = matched_assets
            .iter()
            .filter(|&asset| asset.browser_download_url.is_some())
            .collect();

        if selected.is_empty() {
            return Err(anyhow::anyhow!("❌ No assets with download URL found."));
        } else {
            println!("⚠️  Defaulting to the first asset ({})", selected[0].name);
            url = selected[0].browser_download_url.as_ref().unwrap().clone();
        }
    } else {
        if matched_assets[0].browser_download_url.is_none() {
            return Err(anyhow::anyhow!("❌ No download URL found"));
        }
        url = matched_assets[0]
            .browser_download_url
            .as_ref()
            .unwrap()
            .clone();
    }

    Ok(url)
}

/// Downloads a file from the given URL and saves it to the specified destination path.
///
/// # Arguments
///
/// * `url` - The URL of the file to download.
/// * `dest_path` - The destination path where the file should be saved.
///
/// # Returns
///
/// A `Result` containing the path of the downloaded file or an error.
async fn download_file(url: &str, dest_path: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "gh-app-installer/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download {}: HTTP {}",
            url,
            response.status()
        ));
    }

    let bytes = response.bytes().await?;

    // Create parent directories if they don't exist
    if let Some(parent) = std::path::Path::new(dest_path).parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(dest_path, bytes)?;

    Ok(dest_path.to_string())
}

async fn download_and_extract(url: &str, temp_path: &Path) -> Result<()> {
    // Download and extract
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "gh-app-installer/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download update: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;
    //
    // Extract archive based on URL extension
    println!("ℹ️  Temporary folder {}", temp_path.display());
    if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
        extract_tar_gz(&bytes, temp_path)?;
    } else if url.ends_with(".tar") {
        extract_tar(&bytes, temp_path)?;
    } else if url.ends_with(".zip") {
        extract_zip(&bytes, temp_path)?;
    } else {
        return Err(anyhow::anyhow!("Unsupported archive format"));
    }
    // show extracted files
    println!("ℹ️  Extracted files:");
    for entry in fs::read_dir(temp_path)? {
        let entry = entry?;
        if let Some(name) = entry.path().file_name() {
            println!("    - {}", name.to_string_lossy());
        }
    }

    Ok(())
}

/// Update gh-app-installer to the latest version
///
/// Returns an error if the update fails.
///
async fn self_update(dry_run: bool) -> Result<()> {
    println!("🔍 Checking for updates to gh-app-installer...");
    let (status, release) = get_thisapp_status_and_release().await?;

    println!("{}", status);

    // Parse versions for comparison
    let latest_version = status.latest_version.unwrap();
    let current_version = status.current_version.unwrap();
    let latest_version_parsed = Version::parse(&latest_version)
        .with_context(|| format!("Invalid latest version: {}", latest_version))?;
    let current_version_parsed = Version::parse(&current_version)
        .with_context(|| format!("Invalid current version: {}", current_version))?;

    // Check if the latest version is newer than the current version
    if latest_version_parsed <= current_version_parsed {
        if latest_version_parsed == current_version_parsed {
            println!(
                "✅ gh-app-installer is already at the latest version (v{})",
                current_version
            );
        } else {
            println!(
                "ℹ️  Local version (v{}) is newer than the latest release (v{})",
                current_version, latest_version
            );
        }
        return Ok(());
    }
    println!(
        "🆕 Updating gh-app-installer v{} -> v{}",
        current_version, latest_version
    );

    if dry_run {
        println!("🔄 [DRY RUN] Would update binary");
    }

    // Get current executable path
    let current_exe = std::env::current_exe().context("Failed to get current executable path")?;

    // Check if we can write to the current executable
    if let Err(e) = fs::metadata(&current_exe).and_then(|m| {
        if m.permissions().readonly() {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Binary is read-only",
            ))
        } else {
            Ok(())
        }
    }) {
        return Err(anyhow::anyhow!(
            "❌ Cannot update: insufficient permissions to replace binary at {}\n   Error: {}\n   Hint: Try running with elevated permissions or reinstall manually",
            current_exe.display(),
            e
        ));
    }

    // get the first asset that matches with the platform with a valid download URL
    let url = get_best_url(&release)?;

    if dry_run {
        println!("   📥 [DRY RUN] Would Downloading from {}", url);
        return Ok(());
    } else {
        println!("   📥  Downloading from {}", url);
    }

    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Download and extract
    download_and_extract(&url, &temp_path).await?;

    // Find the new binary
    let new_binary_path = find_binary_in_extracted(temp_path, "rs-gh-app")
        .or_else(|_| find_binary_in_extracted(temp_path, "gh-app-installer"))
        .context("Could not find updated binary in downloaded archive")?;

    // Replace current binary and set permissions
    println!("   🔄 Replacing current binary...");

    let backup_path: PathBuf;

    // On Windows, we might need to rename the current exe first
    #[cfg(windows)]
    {
        backup_path = current_exe.with_extension("exe.old");
        if backup_path.exists() {
            fs::remove_file(&backup_path)?;
        }
        fs::rename(&current_exe, &backup_path)?;
        fs::copy(&new_binary_path, &current_exe)?;
        // Clean up backup on successful replacement
        let _ = fs::remove_file(&backup_path);
    }

    #[cfg(not(windows))]
    {
        backup_path = current_exe.with_extension(".old");
        if backup_path.exists() {
            fs::remove_file(&backup_path)?;
        }
        fs::rename(&current_exe, &backup_path)?;
        fs::copy(&new_binary_path, &current_exe)?;
        // Make executable
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&current_exe, perms)?;
        // Clean up backup on successful replacement
        let _ = fs::remove_file(&backup_path);
    }

    println!(
        "✅ Successfully updated gh-app-installer to v{}",
        latest_version
    );
    println!("🎉 Run the command again to use the new version");

    Ok(())
}

/**
 * Check the status of the given apps.
 *
 * If `stop_on_error` is `true`, the function will stop checking apps if an error occurs.
 * If `stop_on_error` is `false`, the function will continue checking apps even if an error occurs.
 */
async fn check_apps(apps: Vec<App>, stop_on_error: bool, debug: bool) -> Result<()> {
    for app in apps {
        match get_app_status_and_release(&app, debug).await {
            Ok((status, _)) => {
                println!("{}", status);
            }
            Err(e) => {
                println!("❌ Failed to get status for {}: {}", &app.name.clone(), e);
                if stop_on_error {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

/// Download an app from the given URL and install it.
///
/// Sets the permissions to executable if necessary.
async fn download_and_install(app: &App, url: &str) -> Result<()> {
    let bin_dir = get_bin_dir()?;

    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Download and extract
    download_and_extract(&url, &temp_path).await?;

    // Find and move binary
    let binary_path = find_binary_in_extracted(temp_path, &app.bin)?;
    let target_path = bin_dir.join(&app.bin);

    fs::copy(&binary_path, &target_path)?;
    println!(
        "ℹ️  moved {} to {}",
        binary_path.display(),
        target_path.display()
    );

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target_path, perms)?;
    }

    Ok(())
}

/// Process a template string by replacing placeholders with actual values.
///
/// # Arguments
/// * `template` - The template string to process.
/// * `app` - The application information.
/// * `version` - The version of the application.
///
/// # Returns
/// A `Result` containing the processed template string or an error.
///
/// # Examples
/// ```
/// let template = "Hello, {{name}} {{version}} on {{os}}-{{arch}}!";
/// ```
async fn process_template(template: &str, app: &App, version: &str) -> Result<String> {
    let bin_dir = get_bin_dir()?;
    let bin_path = bin_dir.join(&app.bin);
    let app_path = std::env::current_dir()?;

    // get system info
    let raw_os = std::env::consts::OS;
    let raw_arch = std::env::consts::ARCH;

    let (normalized_os, normalized_arch, suffix) = match (raw_os, raw_arch) {
        ("linux", "x86_64") => ("linux", "x86_64", "x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => ("linux", "aarch64", "aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => ("darwin", "x86_64", "x86_64-apple-darwin"),
        ("macos", "aarch64") => ("darwin", "aarch64", "aarch64-apple-darwin"),
        ("windows", "x86_64") => ("windows", "x86_64", "x86_64-pc-windows-msvc"),
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported platform: {}-{}",
                raw_os,
                raw_arch
            ));
        }
    };

    let mut variables = HashMap::new();
    variables.insert("name".to_string(), app.name.clone());
    variables.insert("bin".to_string(), app.bin.clone());
    variables.insert("version".to_string(), version.to_string());
    variables.insert("os".to_string(), normalized_os.to_string());
    variables.insert("arch".to_string(), normalized_arch.to_string());
    variables.insert("raw_os".to_string(), raw_os.to_string());
    variables.insert("raw_arch".to_string(), raw_arch.to_string());
    variables.insert("suffix".to_string(), suffix.to_string());
    variables.insert("bin_dir".to_string(), bin_dir.display().to_string());
    variables.insert("bin_path".to_string(), bin_path.display().to_string());
    variables.insert("app_path".to_string(), app_path.display().to_string());

    let mut result = template.to_string();

    // Process download functions first
    result = process_download_functions(&result).await?;

    // Then process regular variables
    for (key, value) in variables {
        result = result.replace(&format!("{{{}}}", key), &value);
    }

    Ok(result)
}

/// Process download functions in the template.
///
/// This function replaces all occurrences of `{download(url, dest_path)}` with the path to the downloaded file.
///
/// # Arguments
///
/// * `template` - The template string containing download functions.
///
/// # Returns
///
/// A `Result` containing the processed template string or an error.
async fn process_download_functions(template: &str) -> Result<String> {
    let download_regex = Regex::new(r"\{download\(([^,]+),\s*([^)]+)\)\}").unwrap();
    let mut result = template.to_string();

    for cap in download_regex.captures_iter(template) {
        let full_match = &cap[0];
        let url = cap[1].trim();
        let dest_path = cap[2].trim();

        // Download the file
        let downloaded_path = download_file(url, dest_path).await?;

        // Replace the download function with the path
        result = result.replace(full_match, &downloaded_path);
    }

    Ok(result)
}

/// Execute install/update commands for the given app.
///
/// # Arguments
///
/// * `app` - The app to execute commands for.
/// * `version` - The version of the app.
/// * `is_update` - Whether to execute an update command.
/// * `dry_run` - Whether to execute the command or just print it.
///
async fn execute_app_commands(
    app: &App,
    version: &str,
    is_update: bool,
    dry_run: bool,
    debug: bool,
) -> Result<()> {
    let (command, log) = if is_update && app.update_command.is_some() {
        (app.update_command.as_ref().unwrap(), "update")
    } else {
        (app.install_command.as_ref().unwrap(), "install")
    };

    let processed_command = process_template(command, app, version).await?;

    if dry_run {
        println!(
            "   ⚙️ [DRY RUN] Would execute {} command for {} \n\t {} ",
            log, app.name, processed_command
        );
        return Ok(());
    }
    if debug {
        println!(
            "🩺 [DEBUG] Executing {} command for {} \n{}\n🩺 [DEBUG] -- ",
            log, app.name, processed_command
        );
    }

    let output = Command::new("sh")
        .arg("-c")
        .arg(&processed_command)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "\n    Command: {}\n    Error:     {}",
            processed_command,
            stderr
        ));
    }

    Ok(())
}

/// Install the given app.
///
/// If `dry_run` is `true`, the function will only print the installation steps without actually installing the app.
///
/// # Arguments
///
/// * `app` - The app to install.
/// * `dry_run` - Whether to perform a dry run.
///
/// # Errors
///
/// This function will return an error if the app cannot be installed.
async fn install_app(app: &App, dry_run: bool, debug: bool) -> Result<()> {
    let (status, release) = get_app_status_and_release(app, debug).await?;

    if status.pixi_managed.unwrap_or(false) {
        println!("{}", status);
        return Ok(());
    }

    if !status.is_version_update_needed() {
        println!("{}", status);
        return Ok(());
    }

    let latest_version = status.latest_version.unwrap();
    let is_update = status.current_version.is_some();

    if dry_run {
        println!(
            "🔍 [DRY RUN] Would {} {} v{}",
            if is_update { "update" } else { "install" },
            app.name,
            latest_version
        );
    }

    println!(
        "🔄 {} {} v{}",
        if is_update { "Updating" } else { "Installing" },
        app.name,
        latest_version
    );

    match app.installation_method() {
        InstallationMethod::GitHub => {
            let url = get_best_url(&release)?;
            if dry_run {
                println!("   📥 [DRY RUN] Would Downloading from {}", url);
                println!(
                    "   📦 [DRY RUN] Would extract and install binary to: {}",
                    get_bin_dir()?.display()
                );
            } else {
                println!("   📥  Downloading from {}", url);
                download_and_install(app, &url).await?;
            }
        }
        InstallationMethod::Commands => {
            execute_app_commands(app, &latest_version, is_update, dry_run, debug).await?;
        }
    }

    // Verify installation
    if !dry_run {
        if let Some(version) = app::get_current_version_with_debug(&app.bin, debug) {
            println!("✅ {} v{} installed successfully", app.name, version);
        } else {
            println!(
                "⚠️  {} installed but version not detectable (binary may not support standard version flags)",
                app.name
            );
        }
    } else {
        println!(
            "   ℹ️ [DRY RUN] Would check if {} installed successfully",
            app.name
        );
    }

    Ok(())
}

/// Install the given apps.
///
/// If `dry_run` is `true`, the function will only print the installation commands without actually installing the apps.
/// If `stop_on_error` is `true`, the function will stop installing apps if an error occurs.
/// If `stop_on_error` is `false`, the function will continue installing apps even if an error occurs.
async fn install_apps(
    apps: Vec<App>,
    dry_run: bool,
    stop_on_error: bool,
    debug: bool,
) -> Result<()> {
    for app in apps {
        let result = install_app(&app, dry_run, debug).await;

        if let Err(e) = result {
            eprintln!("❌ Failed to install {}: {}", app.name, e);
            if stop_on_error {
                return Err(e);
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = load_config(&cli.config).await?;

    if cli.debug {
        // Check current PATH
        if let Ok(path_var) = env::var("PATH") {
            println!("🩺 [DEBUG] Current PATH: {}", path_var);
        }
    }

    match cli.command {
        Commands::Install { app_name, dry_run } => {
            let apps = filter_apps(&config.apps, app_name)?;
            install_apps(apps, dry_run, cli.stop_on_error, cli.debug).await?;
        }
        Commands::Check { app_name } => {
            let apps = filter_apps(&config.apps, app_name)?;
            check_apps(apps, cli.stop_on_error, cli.debug).await?;
        }
        Commands::SelfUpdate { dry_run } => {
            self_update(dry_run).await?;
        }
    }

    Ok(())
}
