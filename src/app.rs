use anyhow::Result;
use regex::Regex;
/// Defines application information and its details.
use serde::{Deserialize, Serialize};
use std::fmt;
use std::process::Command;

/// Represents an application with its details.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct App {
    pub name: String,
    pub bin: String,
    pub repo: Option<String>,
    pub template: Option<String>,
    pub install_command: Option<String>,
    pub update_command: Option<String>,
    pub script: Option<String>,
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

#[derive(Debug)]
pub enum InstallationMethod {
    Template, // Standard GitHub release download
    Commands, // Custom install/update commands
    Script,   // Custom script execution
}

// app information
#[derive(Debug)]
pub struct AppStatus {
    pub app: App,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub needs_install: bool,
    pub pixi_managed: bool,
}

// implement methods of App
impl App {
    /**
     * Get the template for the app archive filename
     */
    pub fn get_template(&self) -> String {
        self.template
            .clone()
            .unwrap_or_else(|| "{bin}-v{version}-{suffix}.tar.gz".to_string())
    }

    /**
     * Check if the app has a repository
     */
    pub fn has_repo(&self) -> bool {
        self.repo.is_some()
    }

    /**
     * Get the repository short-URL for the app
     */
    pub fn get_repo(&self) -> &str {
        self.repo.as_ref().map_or("", |v| v)
    }

    /**
     * Get the installation method for the app whether it is a command a script
     * or a github template
     */
    pub fn installation_method(&self) -> InstallationMethod {
        if self.install_command.is_some() || self.update_command.is_some() {
            InstallationMethod::Commands
        } else if self.script.is_some() {
            InstallationMethod::Script
        } else {
            InstallationMethod::Template
        }
    }
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
pub fn filter_apps(apps: &[App], app_name: Option<String>) -> Result<Vec<App>> {
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

/// Get the current version of the given binary.
///
/// # Arguments
/// * `bin_name` - The name of the binary to check.
///
/// # Returns
/// The current version of the binary, or None if it could not be determined.
pub fn get_current_version(bin_name: &str) -> Option<String> {
    get_current_version_with_debug(bin_name, false)
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
