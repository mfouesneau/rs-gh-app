use regex::Regex;
use semver::Version;
/// Defines application information and its details.
use serde::{Deserialize, Serialize};
use std::fmt;
use std::process::Command;

/// Represents an application with its details.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct App {
    pub name: String,
    pub bin: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub install_command: Option<String>,
    pub update_command: Option<String>,
    pub version_command: Option<String>,
}

// app information
#[derive(Debug)]
pub struct AppStatus {
    pub app: App,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub pixi_managed: Option<bool>,
}

#[derive(Debug)]
pub enum InstallationMethod {
    GitHub,   // Direct download from GitHub releases
    Commands, // Custom install/update commands
}

impl fmt::Display for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}\n
             bin: {}\n
             repo: {}",
            self.name,
            self.bin,
            self.repo.as_ref().map_or("unknown", |v| v)
        )
    }
}

/// * Print the status of the given app.
///  *
///  * If `pixi_managed` is `true`, the function will print a message indicating that the app is managed by pixi.
///  * If `pixi_managed` is `false`, the function will print a message indicating the current and latest versions of the app.
impl fmt::Display for AppStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_pixi_managed() {
            return match &self.current_version {
                Some(version) => write!(f, "ℹ️  {} ({}) [pixi managed]", self.app.name, version),
                None => write!(f, "ℹ️  {} [pixi managed]", self.app.name),
            };
        }

        match (&self.current_version, &self.latest_version) {
            (Some(current), Some(latest)) => {
                if self.is_version_update_needed() {
                    write!(
                        f,
                        "🆕 {} v{} -> v{} (update available)",
                        self.app.name, current, latest
                    )
                } else {
                    write!(
                        f,
                        "✅ {} is already at the latest version ({})",
                        self.app.name, current
                    )
                }
            }
            (None, Some(latest)) => {
                write!(
                    f,
                    "📦 {} v{} (not installed or version not detectable)",
                    self.app.name, latest
                )
            }
            (Some(current), None) => {
                write!(
                    f,
                    "❓ {} v{} (could not check for updates or latest version not detectable)",
                    self.app.name, current
                )
            }
            _ => {
                write!(f, "❓ {} (version unknown)", self.app.name)
            }
        }
    }
}

// implement methods of App
impl App {
    /**
     * Get the repository short-URL for the app
     */
    pub fn get_repo(&self) -> &str {
        self.repo.as_ref().map_or("", |v| v)
    }

    /**
     * Get the installation method for the app whether it is a command
     * or a github template
     */
    pub fn installation_method(&self) -> InstallationMethod {
        if self.install_command.is_some() || self.update_command.is_some() {
            InstallationMethod::Commands
        } else {
            InstallationMethod::GitHub
        }
    }
}

/// Check if the given binary is managed by pixi.
///
/// # Arguments
/// * `bin_name` - The name of the binary to check.
///
/// # Returns
/// A boolean indicating whether the binary is managed by pixi.
pub fn check_pixi_managed(bin_name: &str) -> bool {
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

impl AppStatus {
    pub fn new(app: &App) -> Self {
        Self {
            pixi_managed: Some(check_pixi_managed(&app.bin)),
            current_version: get_current_version_with_debug(&app.bin, false),
            latest_version: None,
            app: app.clone(),
        }
    }
    pub fn is_pixi_managed(&self) -> bool {
        self.pixi_managed.clone().unwrap_or(false)
    }
    pub fn set_latest_version(&mut self, version: String) {
        self.latest_version = Some(version);
    }

    /// Check if a version update is needed.
    ///
    /// This function compares the current version with the latest version.
    /// If the latest version is greater than the current version, an update is needed.
    /// If the versions cannot be parsed as semantic versions, a string comparison is used.
    ///
    /// Returns `true` if an update is needed, `false` otherwise.
    pub fn is_version_update_needed(&self) -> bool {
        match (&self.current_version, &self.latest_version) {
            (None, None) => false,   // No idea, so do nothing
            (None, Some(_)) => true, // Not installed, so update needed
            (Some(current_ver), Some(latest_ver)) => {
                // Try to parse both versions as semantic versions
                match (Version::parse(current_ver), Version::parse(latest_ver)) {
                    (Ok(current_semver), Ok(latest_semver)) => latest_semver > current_semver,
                    _ => {
                        // Fall back to string comparison if parsing fails
                        current_ver != latest_ver
                    }
                }
            }
            (Some(_), None) => {
                // Not installed, no update information
                false
            }
        }
    }
}

/// Get the current version of the given binary.
///
/// # Arguments
/// * `bin_name` - The name of the binary to check.
/// * `debug` - Whether to print debug information.
///
/// # Returns
/// The current version of the binary, or None if it could not be determined.
pub fn get_current_version_with_debug(bin_name: &str, debug: bool) -> Option<String> {
    // Try different version flags in order of preference
    let version_flags = ["--version", "-V", "-v", "version"];

    for flag in &version_flags {
        if let Ok(output) = Command::new(bin_name).arg(flag).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Try to extract version from stdout first, then stderr
                if let Some(version) = extract_version_from_string(&stdout) {
                    if debug {
                        println!(
                            "🔍 Version detected using '{} {}': {}",
                            bin_name, flag, version
                        );
                    }
                    return Some(version);
                }
                if let Some(version) = extract_version_from_string(&stderr) {
                    if debug {
                        println!(
                            "🔍 Version detected using '{} {}' (from stderr): {}",
                            bin_name, flag, version
                        );
                    }
                    return Some(version);
                }
            }
        }
    }

    // If no version flag worked, try running the command without arguments
    // Some apps print version info in help output
    if let Ok(output) = Command::new(bin_name).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if let Some(version) = extract_version_from_string(&stdout) {
            if debug {
                println!(
                    "🔍 Version detected from '{} {}' help output: {}",
                    bin_name, "(no args)", version
                );
            }
            return Some(version);
        }
        if let Some(version) = extract_version_from_string(&stderr) {
            if debug {
                println!(
                    "🔍 Version detected from '{} {}' help output (stderr): {}",
                    bin_name, "(no args)", version
                );
            }
            return Some(version);
        }
    }

    if debug {
        println!(
            "⚠️  Could not detect version for '{}' using any method",
            bin_name
        );
    }
    None
}

/// Parse version from string - handles various version formats
pub fn extract_version_from_string(s: &str) -> Option<String> {
    // Try different version patterns in order of preference
    let patterns = [
        r"(\d{1,5}\.\d{1,5}\.\d{1,5}(?:\.\d{1,5})?)", // x.y.z or x.y.z.w
        r"v(\d{1,5}\.\d{1,5}\.\d{1,5}(?:\.\d{1,5})?)", // v-prefixed versions
        r"version\s+(\d{1,5}\.\d{1,5}\.\d{1,5}(?:\.\d{1,5})?)", // "version x.y.z"
        r"(\d{1,5}\.\d{1,5})",                        // x.y (two-part versions)
    ];

    for pattern in &patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(cap) = re.captures(s) {
                if let Some(version) = cap.get(1) {
                    return Some(version.as_str().to_string());
                }
            }
        }
    }
    None
}
