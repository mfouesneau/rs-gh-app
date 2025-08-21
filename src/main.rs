use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

// CLI options ==============================================

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
}

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

// app.yaml format =================================================

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    apps: Vec<App>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct App {
    name: String,
    bin: String,
    repo: Option<String>,
    template: Option<String>,
    install_command: Option<String>,
    update_command: Option<String>,
    script: Option<String>,
}

// Internal information ============================================

// keep track of the system information
#[derive(Debug)]
struct SystemInfo {
    os: String,
    arch: String,
    suffix: String,
}

// app information
#[derive(Debug)]
struct AppStatus {
    app: App,
    current_version: Option<String>,
    latest_version: Option<String>,
    needs_install: bool,
    pixi_managed: bool,
}

// implement methods of App
impl App {
    /**
     * Get the template for the app archive filename
     */
    fn get_template(&self) -> String {
        self.template
            .clone()
            .unwrap_or_else(|| "{bin}-v{version}-{suffix}.tar.gz".to_string())
    }

    /**
     * Check if the app has a repository
     */
    fn has_repo(&self) -> bool {
        self.repo.is_some()
    }

    /**
     * Get the repository short-URL for the app
     */
    fn get_repo(&self) -> &str {
        self.repo.as_ref().map_or("", |v| v)
    }

    /**
     * Get the installation method for the app whether it is a command a script
     * or a github template
     */
    fn installation_method(&self) -> InstallationMethod {
        if self.install_command.is_some() || self.update_command.is_some() {
            InstallationMethod::Commands
        } else if self.script.is_some() {
            InstallationMethod::Script
        } else {
            InstallationMethod::Template
        }
    }
}

#[derive(Debug)]
enum InstallationMethod {
    Template, // Standard GitHub release download
    Commands, // Custom install/update commands
    Script,   // Custom script execution
}

// CLI main definition ==============================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = load_config(&cli.config).await?;
    let system_info = detect_system_info()?;

    match cli.command {
        Commands::Install { app_name, dry_run } => {
            let apps = filter_apps(&config.apps, app_name)?;
            install_apps(apps, &system_info, dry_run, cli.stop_on_error).await?;
        }
        Commands::Check { app_name } => {
            let apps = filter_apps(&config.apps, app_name)?;
            check_apps(apps, &system_info, cli.stop_on_error).await?;
        }
        Commands::SelfUpdate { dry_run } => {
            self_update(&system_info, dry_run).await?;
        }
    }
    Ok(())
}

/**
 * Load the configuration from the given file.
 *
 * This function reads the configuration file and returns a `Config` struct.
 * If the file does not exist, it creates a sample configuration file.
 */
async fn load_config(config_file: &str) -> Result<Config> {
    let config_path = PathBuf::from(config_file);

    if !config_path.exists() {
        // Create a sample config file
        let sample_config = Config {
            apps: vec![
                App {
                    name: "dust".to_string(),
                    bin: "dust".to_string(),
                    repo: Some("bootandy/dust".to_string()),
                    template: Some("{bin}-v{version}-{suffix}.tar.gz".to_string()),
                    install_command: None,
                    update_command: None,
                    script: None,
                },
                App {
                    name: "bat".to_string(),
                    bin: "bat".to_string(),
                    repo: Some("sharkdp/bat".to_string()),
                    template: Some("{bin}-v{version}-{suffix}.tar.gz".to_string()),
                    install_command: None,
                    update_command: None,
                    script: None,
                },
                App {
                    name: "uv".to_string(),
                    bin: "uv".to_string(),
                    repo: Some("astral-sh/uv".to_string()),
                    template: None,
                    install_command: Some("{download(https://astral.sh/uv/install.sh, /tmp/uv-install.sh)} && sh /tmp/uv-install.sh --bin-dir {bin_dir} --yes".to_string()),
                    update_command: Some("{bin_path} self update".to_string()),
                    script: None,
                },
            ],
        };

        let yaml = serde_yaml::to_string(&sample_config)?;
        fs::write(&config_path, yaml)?;
        println!("📝 Created sample config file: {}", config_path.display());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    let config: Config =
        serde_yaml::from_str(&content).with_context(|| "Failed to parse YAML config")?;

    Ok(config)
}

/**
 * Filter the apps based on the given app name.
 *
 * If `app_name` is `None`, all apps are returned.
 * If `app_name` is `Some(name)`, the app with the given name is returned.
 */
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

/**
 * Detect the system information.
 *
 * This function detects the operating system and architecture of the system.
 * It returns a `SystemInfo` struct containing the normalized OS, architecture, and suffix.
 */
fn detect_system_info() -> Result<SystemInfo> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let (normalized_os, normalized_arch, suffix) = match (os, arch) {
        ("linux", "x86_64") => ("linux", "x86_64", "x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => ("linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => ("darwin", "x86_64", "x86_64-apple-darwin"),
        ("macos", "aarch64") => ("darwin", "aarch64", "x86_64-apple-darwin"), // Many releases use x86_64 for Mac
        ("windows", "x86_64") => ("windows", "x86_64", "x86_64-pc-windows-msvc"),
        _ => return Err(anyhow::anyhow!("Unsupported platform: {}-{}", os, arch)),
    };

    Ok(SystemInfo {
        os: normalized_os.to_string(),
        arch: normalized_arch.to_string(),
        suffix: suffix.to_string(),
    })
}

/**
 * Check the status of the given apps.
 *
 * If `stop_on_error` is `true`, the function will stop checking apps if an error occurs.
 * If `stop_on_error` is `false`, the function will continue checking apps even if an error occurs.
 */
async fn check_apps(apps: Vec<App>, system_info: &SystemInfo, stop_on_error: bool) -> Result<()> {
    for app in apps {
        match get_app_status(&app, system_info).await {
            Ok(status) => print_app_status(&status),
            Err(e) => {
                eprintln!("❌ Failed to check {}: {}", app.name, e);
                if stop_on_error {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

/**
 * Install the given apps.
 *
 * If `dry_run` is `true`, the function will only print the installation commands without actually installing the apps.
 * If `stop_on_error` is `true`, the function will stop installing apps if an error occurs.
 * If `stop_on_error` is `false`, the function will continue installing apps even if an error occurs.
 */
async fn install_apps(
    apps: Vec<App>,
    system_info: &SystemInfo,
    dry_run: bool,
    stop_on_error: bool,
) -> Result<()> {
    for app in apps {
        let result = install_app(&app, system_info, dry_run).await;

        if let Err(e) = result {
            eprintln!("❌ Failed to install {}: {}", app.name, e);
            if stop_on_error {
                return Err(e);
            }
        }
    }
    Ok(())
}

/**
 * Get the status of the given app.
 *
 * If `stop_on_error` is `true`, the function will stop checking apps if an error occurs.
 * If `stop_on_error` is `false`, the function will continue checking apps even if an error occurs.
 */
async fn get_app_status(app: &App, _system_info: &SystemInfo) -> Result<AppStatus> {
    let pixi_managed = check_pixi_managed(&app.bin);
    let current_version = get_current_version(&app.bin);

    let latest_version = if app.has_repo() {
        get_latest_version(app.get_repo()).await?
    } else {
        // For apps without repo (custom commands/scripts), we can't check versions
        current_version
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    };

    let needs_install = !pixi_managed
        && (current_version.is_none()
            || (app.has_repo() && current_version.as_ref() != Some(&latest_version)));

    Ok(AppStatus {
        app: app.clone(),
        current_version,
        latest_version: Some(latest_version),
        needs_install,
        pixi_managed,
    })
}

/**
 * Print the status of the given app.
 *
 * If `pixi_managed` is `true`, the function will print a message indicating that the app is managed by pixi.
 * If `pixi_managed` is `false`, the function will print a message indicating the current and latest versions of the app.
 */
fn print_app_status(status: &AppStatus) {
    if status.pixi_managed {
        match &status.current_version {
            Some(version) => println!("ℹ️  {} ({}) [pixi managed]", status.app.name, version),
            None => println!("ℹ️  {} [pixi managed]", status.app.name),
        }
        return;
    }

    match (&status.current_version, &status.latest_version) {
        (Some(current), Some(latest)) => {
            if current == latest {
                println!(
                    "✅ {} is already at the latest version ({})",
                    status.app.name, latest
                );
            } else {
                println!(
                    "🆕 {} v{} -> v{} (update available)",
                    status.app.name, current, latest
                );
            }
        }
        (None, Some(latest)) => {
            println!("📦 {} v{} (not installed)", status.app.name, latest);
        }
        _ => {
            println!("❓ {} (version unknown)", status.app.name);
        }
    }
}

/**
 * Installs an app.
 */
async fn install_app(app: &App, system_info: &SystemInfo, dry_run: bool) -> Result<()> {
    let status = get_app_status(app, system_info).await?;

    if status.pixi_managed {
        print_app_status(&status);
        return Ok(());
    }

    if !status.needs_install {
        print_app_status(&status);
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
        preview_installation_steps(app, &latest_version, system_info, is_update).await?;
        return Ok(());
    }

    println!(
        "🔄 {} {} v{}",
        if is_update { "Updating" } else { "Installing" },
        app.name,
        latest_version
    );

    match app.installation_method() {
        InstallationMethod::Template => {
            let url = build_download_url(app, &latest_version, system_info)?;
            println!("ℹ️  Downloading from {}", url);
            download_and_install(app, &url).await?;
        }
        InstallationMethod::Commands => {
            execute_app_commands(app, &latest_version, system_info, is_update).await?;
        }
        InstallationMethod::Script => {
            execute_app_script(app, &latest_version, system_info).await?;
        }
    }

    // Verify installation
    if let Some(version) = get_current_version(&app.bin) {
        println!("✅ {} v{} installed successfully", app.name, version);
    } else {
        println!("⚠️  {} installed but version check failed", app.name);
    }

    Ok(())
}

/**
 * Builds the download URL for an app.
 */
fn build_download_url(app: &App, version: &str, system_info: &SystemInfo) -> Result<String> {
    let template = app.get_template();
    let filename = template
        .replace("{name}", &app.name)
        .replace("{bin}", &app.bin)
        .replace("{version}", version)
        .replace("{os}", &system_info.os)
        .replace("{arch}", &system_info.arch)
        .replace("{suffix}", &system_info.suffix);

    Ok(format!(
        "https://github.com/{}/releases/download/{}/{}",
        app.get_repo(),
        version,
        filename
    ))
}

/**
 * Downloads and installs an app.
 */
async fn download_and_install(app: &App, url: &str) -> Result<()> {
    let bin_dir = get_bin_dir()?;

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "gh-app-installer/0.1.0")
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download: HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await?;

    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

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

// Deal with archives ======================================================

fn extract_tar_gz(bytes: &[u8], dest_path: &std::path::Path) -> Result<()> {
    let tar = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(tar);
    archive.unpack(dest_path)?;
    Ok(())
}

fn extract_tar(bytes: &[u8], dest_path: &std::path::Path) -> Result<()> {
    let mut archive = tar::Archive::new(bytes);
    archive.unpack(dest_path)?;
    Ok(())
}

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

fn get_bin_dir() -> Result<PathBuf> {
    if let Ok(bin_dir) = std::env::var("bin_dir") {
        Ok(PathBuf::from(bin_dir))
    } else {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(".local").join("bin"))
    }
}

// Pixi-related functions ======================================================

fn check_pixi_managed(bin_name: &str) -> bool {
    if !Command::new("pixi").arg("--version").output().is_ok() {
        return false;
    }

    let output = Command::new("pixi")
        .args(["global", "list", bin_name])
        .output();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        !stdout.contains("No global environments found")
    } else {
        false
    }
}

// Get current / latest versions ===============================================

fn get_current_version(bin_name: &str) -> Option<String> {
    let output = Command::new(bin_name).arg("--version").output().ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    extract_version_from_string(&stdout)
}

/**
 * Get the latest version of a binary from its GitHub repository.
 *
 * Returns the latest version of the binary.
 *
 * Check rate limit on github first. (useful for avoiding rate limit errors)
 */
async fn get_latest_version(repo: &str) -> Result<String> {
    let client = reqwest::Client::new();

    // Check rate limit first
    let rate_limit_response = client
        .get("https://api.github.com/rate_limit")
        .header("User-Agent", "gh-app-installer/0.1.0")
        .send()
        .await?;

    if !rate_limit_response.status().is_success() {
        println!("⚠️  Could not check rate limit, proceeding anyway");
    } else {
        let rate_limit_text = rate_limit_response.text().await?;
        if let Ok(rate_limit) = serde_json::from_str::<serde_json::Value>(&rate_limit_text) {
            let remaining = rate_limit["rate"]["remaining"].as_u64().unwrap_or(1);
            if remaining == 0 {
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

                return Err(anyhow::anyhow!(
                    "🚨 GitHub API rate limit exceeded. Resets at: {} ({})",
                    reset_datetime.format("%Y-%m-%d %H:%M:%S UTC"),
                    delta_str
                ));
            }
        }
    }

    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let response = client
        .get(&url)
        .header("User-Agent", "gh-app-installer/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to fetch latest release: HTTP {}",
            response.status()
        ));
    }

    let response_text = response.text().await?;
    let release: serde_json::Value = serde_json::from_str(&response_text)
        .with_context(|| format!("Failed to parse JSON response: {}", response_text))?;

    let tag_name = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No tag_name found in release"))?;

    extract_version_from_string(tag_name)
        .ok_or_else(|| anyhow::anyhow!("Could not extract version from tag: {}", tag_name))
}

/// Parse version from string defined as xxx.xxx.xxx format
fn extract_version_from_string(s: &str) -> Option<String> {
    let re = Regex::new(r"(\d{1,5}\.\d{1,5}\.\d{1,5})").ok()?;
    re.find(s).map(|m| m.as_str().to_string())
}

/// Preview installation steps for the given version and system information.
async fn preview_installation_steps(
    app: &App,
    version: &str,
    system_info: &SystemInfo,
    is_update: bool,
) -> Result<()> {
    match app.installation_method() {
        InstallationMethod::Template => {
            let url = build_download_url(app, version, system_info)?;
            println!("📥 Would download: {}", url);
            println!(
                "📦 Would extract and install binary to: {}",
                get_bin_dir()?.display()
            );
        }
        InstallationMethod::Commands => {
            let command = if is_update && app.update_command.is_some() {
                app.update_command.as_ref().unwrap()
            } else {
                app.install_command.as_ref().unwrap()
            };

            let processed_command = process_template(command, app, version, system_info).await?;
            println!("🔧 Would run: {}", processed_command);
        }
        InstallationMethod::Script => {
            let script = app.script.as_ref().unwrap();
            let processed_script = process_template(script, app, version, system_info).await?;
            println!("📜 Would execute script: {}", processed_script);
        }
    }
    Ok(())
}

// Installation methods ====================================================================
/// - Commands: Run a command to install the application.
/// - Script: Execute a script to install the application.
/// - Download: Download the application from a URL and install it.

async fn execute_app_commands(
    app: &App,
    version: &str,
    system_info: &SystemInfo,
    is_update: bool,
) -> Result<()> {
    let command = if is_update && app.update_command.is_some() {
        println!("🔄 Running update command...");
        app.update_command.as_ref().unwrap()
    } else {
        println!("🔄 Running install command...");
        app.install_command.as_ref().unwrap()
    };

    let processed_command = process_template(command, app, version, system_info).await?;

    let output = Command::new("sh")
        .arg("-c")
        .arg(&processed_command)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Command failed: {}", stderr));
    }

    Ok(())
}

async fn execute_app_script(app: &App, version: &str, system_info: &SystemInfo) -> Result<()> {
    let script = app.script.as_ref().unwrap();
    let processed_script = process_template(script, app, version, system_info).await?;

    println!("🔄 Executing script...");

    let output = Command::new("sh")
        .arg("-c")
        .arg(&processed_script)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Script failed: {}", stderr));
    }

    Ok(())
}

async fn process_template(
    template: &str,
    app: &App,
    version: &str,
    system_info: &SystemInfo,
) -> Result<String> {
    let bin_dir = get_bin_dir()?;
    let bin_path = bin_dir.join(&app.bin);
    let app_path = std::env::current_dir()?;

    let mut variables = HashMap::new();
    variables.insert("name".to_string(), app.name.clone());
    variables.insert("bin".to_string(), app.bin.clone());
    variables.insert("version".to_string(), version.to_string());
    variables.insert("os".to_string(), system_info.os.clone());
    variables.insert("arch".to_string(), system_info.arch.clone());
    variables.insert("suffix".to_string(), system_info.suffix.clone());
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

async fn self_update(system_info: &SystemInfo, dry_run: bool) -> Result<()> {
    const SELF_REPO: &str = "mfouesneau/rs-gh-app";
    const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

    println!("🔍 Checking for updates to gh-app-installer...");

    // Get latest version from GitHub
    let latest_version = match get_latest_version(SELF_REPO).await {
        Ok(version) => version,
        Err(e) => {
            if e.to_string().contains("404") {
                println!("ℹ️  No releases found on GitHub. This might be a development build.");
                println!("ℹ️  Current version: v{}", CURRENT_VERSION);
                return Ok(());
            } else {
                return Err(e);
            }
        }
    };

    // Parse versions for comparison
    let current_version = Version::parse(CURRENT_VERSION)
        .with_context(|| format!("Invalid current version: {}", CURRENT_VERSION))?;
    let latest_version_parsed = Version::parse(&latest_version)
        .with_context(|| format!("Invalid latest version: {}", latest_version))?;

    if latest_version_parsed <= current_version {
        if latest_version_parsed == current_version {
            println!(
                "✅ gh-app-installer is already at the latest version (v{})",
                CURRENT_VERSION
            );
        } else {
            println!(
                "ℹ️  Local version (v{}) is newer than the latest release (v{})",
                CURRENT_VERSION, latest_version
            );
        }
        return Ok(());
    }

    if dry_run {
        println!(
            "🔍 [DRY RUN] Would update gh-app-installer v{} -> v{}",
            CURRENT_VERSION, latest_version
        );
        let url = build_self_update_url(&latest_version, system_info)?;
        println!("📥 Would download: {}", url);
        println!("🔄 Would replace current binary");
        return Ok(());
    }

    println!(
        "🆕 Updating gh-app-installer v{} -> v{}",
        CURRENT_VERSION, latest_version
    );

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

    // Download new version
    let url = build_self_update_url(&latest_version, system_info)?;
    println!("ℹ️  Downloading from {}", url);

    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Download and extract
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
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

    // Extract based on file extension
    if url.ends_with(".tar.gz") {
        extract_tar_gz(&bytes, temp_path)?;
    } else if url.ends_with(".zip") {
        extract_zip(&bytes, temp_path)?;
    } else {
        return Err(anyhow::anyhow!(
            "Unsupported archive format for self-update"
        ));
    }

    // Find the new binary
    let new_binary_path = find_binary_in_extracted(temp_path, "rs-gh-app")
        .or_else(|_| find_binary_in_extracted(temp_path, "gh-app-installer"))
        .context("Could not find updated binary in downloaded archive")?;

    // Replace current binary
    println!("🔄 Replacing current binary...");

    // On Windows, we might need to rename the current exe first
    #[cfg(windows)]
    {
        let backup_path = current_exe.with_extension("exe.old");
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
        fs::copy(&new_binary_path, &current_exe)?;
        // Make executable
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&current_exe, perms)?;
    }

    println!(
        "✅ Successfully updated gh-app-installer to v{}",
        latest_version
    );
    println!("🎉 Restart your terminal or run the command again to use the new version");

    Ok(())
}

fn build_self_update_url(version: &str, system_info: &SystemInfo) -> Result<String> {
    let archive_ext = if system_info.os == "windows" {
        "zip"
    } else {
        "tar.gz"
    };

    // Following the actual GitHub release pattern: rs-gh-app-{suffix}.{ext}
    let filename = format!("rs-gh-app-{}.{}", system_info.suffix, archive_ext);

    Ok(format!(
        "https://github.com/mfouesneau/rs-gh-app/releases/download/v{}/{}",
        version, filename
    ))
}
