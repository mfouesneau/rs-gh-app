# Static App Manager

A Rust CLI tool for managing installation and updates of GitHub-released or static/standalone applications. This tool automatically detects your system architecture, downloads the appropriate binaries, and keeps your tools up to date.

## Origin

This project was inspired by a personal need for a simple and efficient way to manage GitHub-released applications (eza, bat...) on various platforms while avoiding the sudo command and nitty-gritty details of each machine. Shell scripts work fine, but are a bit cumbersome (and slow).

This app aims to provide a robust solution for users who want to keep their tools up to date without manual intervention. Give it a list of apps you want to install and maintain, and it should do the rest.

## Features

- 🚀 **Automatic Updates**: Check for and install the latest versions of your favorite GitHub-released tools
- 🏷️ **Intelligent Tag Detection**: Automatically detects correct release tags by following GitHub redirects (handles v1.2.3, 1.2.3, release-1.2.3, etc.)
- 🔧 **Flexible Configuration**: YAML-based configuration with multiple installation/update methods (github repo, command, script)
- 🏃 **Dry Run Mode**: Preview what would be installed without actually doing it with verbose step-by-step output
- 🏗️ **Architecture Detection**: Automatically detects your OS and architecture (Linux, macOS, Windows with x86_64/aarch64 support)
- 📦 **Multiple Archive Formats**: Supports tar, tar.gz, and zip archives
- 🔍 **Pixi Integration**: Automatically skips apps managed by pixi (others could be implemented)
- 🛠️ **Custom Commands**: Support for separate install and update commands
- 📥 **Download Function**: Built-in `{download(url, path)}` template function for custom installers
- 📜 **Script Support**: Execute custom installation scripts with full templating support
- ⚡ **GitHub API Integration**: Uses GitHub's API with rate limiting awareness and user-friendly time-to-reset messages

## Installation

### Precompiled Binary Installation

You can download pre-built binaries from the [releases page](https://github.com/mfouesneau/rs-gh-app/releases).

### Manual Installation

1. Clone this repository:
   ```bash
   git clone https://github.com/mfouesneau/rs-gh-app
   cd rs-gh-app
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. Copy the binary to your PATH:
   ```bash
   cp target/release/rs-gh-app ~/.local/bin/
   ```

## Configuration

The tool uses a `apps.yaml` file in the current directory or with the binary by default. You can specify a custom configuration file using the `--config` or `-c` option. If the configuration file doesn't exist, a sample configuration will be created automatically.

### Example Configuration

The following example configuration includes variables and custom installation script usage. (details below)

```yaml
apps:
  # Standard GitHub release apps (backward compatible)
  - name: bottom
    bin: btm
    repo: ClementTsang/bottom
    template: "{name}_{suffix}.tar.gz"

  - name: "zoxide"
    bin: "zoxide"
    repo: "ajeetdsouza/zoxide"
    template: "{bin}-{version}-{suffix}.tar.gz"

  # Custom install/update commands with download function
  - name: "uv"
    bin: "uv"
    repo: "astral-sh/uv"  # For version checking
    install_command: "{download(https://astral.sh/uv/install.sh, /tmp/uv-install.sh)} && sh /tmp/uv-install.sh --bin-dir {bin_dir} --yes"
    update_command: "{bin_path} self update"

  # Script-based installation
  - name: "nerdfonts"
    bin: "nerd-font-patcher"
    script: "{app_path}/scripts/install-nerdfonts.sh {bin_dir} {version}"
```

### Configuration Fields

#### Required Fields
- **name**: Display name for the application
- **bin**: Binary name (used for version checking and as the installed filename)

#### Installation Methods (choose one per entry)

You can mix different installation methods in the same configuration file, allowing you to manage both standard GitHub releases and custom installers in one place.

1. **Standard GitHub Releases**: The traditional method using GitHub releases with customizable URL templates. Perfect for most GitHub projects that follow standard release patterns.
   - **repo**: GitHub repository in format `owner/repo`
   - **template**: URL filename template with placeholders

2. **Custom Commands**: For applications with custom installers (like uv, rustup, etc.) that provide their own installation and update scripts
   - **repo**: (optional) GitHub repository for version checking
   - **install_command**: Command to run for installation
   - **update_command**: (optional) Command to run for updates

3. **Custom Scripts**: For complex installation scenarios that require custom logic. Execute your own scripts with full access to template variables.
   - **script**: Path to script to execute for installation

#### Template Variables

Available in all `template`, `install_command`, `update_command`, and `script` fields:

- `{name}`: Application name
- `{bin}`: Binary name
- `{version}`: Version number (e.g., "1.2.3")
- `{os}`: Operating system ("linux", "darwin", "windows")
- `{arch}`: Architecture ("x86_64", "aarch64")
- `{suffix}`: Combined OS/arch suffix (e.g., "x86_64-unknown-linux-musl")
- `{bin_dir}`: Installation directory path
- `{bin_path}`: Full path to the binary (e.g., "/home/user/.local/bin/app")
- `{app_path}`: Current working directory (useful for script paths)

#### Intelligent Tag Detection

The tool automatically detects the correct release tag format by following GitHub's `/releases/latest` redirect. This handles repositories that use different tagging conventions:
- `v1.2.3` (most common)
- `1.2.3` (without 'v' prefix)
- `release-1.2.3` (custom prefixes)
- Any other tagging scheme

This eliminates the need to guess tag formats and works reliably across different repositories.

#### Special Functions

- `{download(url, destination)}`: Downloads a file and returns the destination path
  - Example: `{download(https://example.com/install.sh, /tmp/install.sh)}`
  - Can be used in command templates for custom installers

#### Integration with Package Managers

The tool automatically detects if an application is managed by [pixi](https://pixi.sh) and will skip installation for pixi-managed applications, showing an informational message instead.

## Usage

### Check Application Versions

Check all configured applications:
```bash
rs-gh-app check
```

Check a specific application:
```bash
rs-gh-app check bat
```

Use a custom configuration file:
```bash
rs-gh-app --config my-apps.yaml check
# or
rs-gh-app -c my-apps.yaml check
```

### Self-Update

Update the tool itself to the latest version:
```bash
rs-gh-app self-update
```

Preview what would be updated:
```bash
rs-gh-app self-update --dry-run
```

Check the current version:
```bash
rs-gh-app --version
```

**Version Comparison**: The self-update feature uses semantic versioning comparison to ensure you only upgrade to newer versions. If your local version is newer than the latest release (e.g., development builds), it will not downgrade and will inform you that your local version is newer.

### Install or Update Applications

Install/update all applications:
```bash
rs-gh-app install
```

Install/update a specific application:
```bash
rs-gh-app install bat
```

Preview what would be installed (dry run):
```bash
rs-gh-app install --dry-run
```

Stop on first error instead of continuing:
```bash
rs-gh-app install --stop-on-error
```

Use a custom configuration file:
```bash
rs-gh-app --config my-apps.yaml install
```

## Installation Directory

By default, binaries are installed to `~/.local/bin`. You can override this by setting the `bin_dir` environment variable:

```bash
export bin_dir="$HOME/my-tools"
rs-gh-app install
```

## Command Line Options

- `--version`: Show the current version of the tool
- `--config, -c <PATH>`: Specify a custom configuration file path (default: `apps.yaml`)
- `--stop-on-error`: Stop on first error instead of continuing with other apps
- `--dry-run`: Preview installation steps without executing them (available for `install` and `self-update` commands)

## Example Output

```
$ rs-gh-app check
✅ dust is already at the latest version (1.2.3)
🆕 bat v0.24.0 -> v0.25.0 (update available)
📦 zoxide v0.9.8 (not installed)
ℹ️  eza [pixi managed]

$ rs-gh-app install --dry-run
✅ dust is already at the latest version (1.2.3)
🔍 [DRY RUN] Would install bat v0.25.0
🏷️  Found release tag: v0.25.0
📥 Would download: https://github.com/sharkdp/bat/releases/download/v0.25.0/bat-v0.25.0-x86_64-apple-darwin.tar.gz
📦 Would extract and install binary to: /Users/user/.local/bin
🔍 [DRY RUN] Would install zoxide v0.9.8
🏷️  Found release tag: v0.9.8
📥 Would download: https://github.com/ajeetdsouza/zoxide/releases/download/v0.9.8/zoxide-0.9.8-x86_64-apple-darwin.tar.gz
📦 Would extract and install binary to: /Users/user/.local/bin
ℹ️  eza [pixi managed]

$ rs-gh-app install uv --dry-run
🔍 [DRY RUN] Would install uv v0.8.12
🔧 Would run: sh /tmp/uv-install.sh --bin-dir /Users/user/.local/bin --yes

$ rs-gh-app install bat
🔄 Installing bat v0.25.0
ℹ️  Downloading from https://github.com/sharkdp/bat/releases/download/v0.25.0/bat-v0.25.0-x86_64-apple-darwin.tar.gz
✅ bat v0.25.0 installed successfully

$ rs-gh-app install uv
🔄 Installing uv v0.8.12
🔄 Running install command...
✅ uv v0.8.12 installed successfully

$ rs-gh-app self-update --dry-run
🔍 Checking for updates to gh-app-installer...
ℹ️  Current version: v0.1.0
🔍 [DRY RUN] Would update gh-app-installer v0.1.0 -> v0.1.1
📥 Would download: https://github.com/mfouesneau/rs-gh-app/releases/download/v0.1.1/rs-gh-app-x86_64-apple-darwin.tar.gz
🔄 Would replace current binary

$ rs-gh-app self-update
🔍 Checking for updates to gh-app-installer...
ℹ️  Current version: v0.1.0
🆕 Updating gh-app-installer v0.1.0 -> v0.1.1
ℹ️  Downloading from https://github.com/mfouesneau/rs-gh-app/releases/download/v0.1.1/rs-gh-app-x86_64-apple-darwin.tar.gz
🔄 Replacing current binary...
✅ Successfully updated gh-app-installer to v0.1.1
🎉 Restart your terminal or run the command again to use the new version

# When local version is newer than latest release
$ rs-gh-app self-update
🔍 Checking for updates to gh-app-installer...
ℹ️  Current version: v0.2.0
ℹ️  Local version (v0.2.0) is newer than the latest release (v0.1.0)
```

## Supported Platforms

- **Linux**: x86_64, aarch64
- **macOS**: x86_64, aarch64 (Apple Silicon)
- **Windows**: x86_64, aarch64  [Not fully tested]

## Error Handling

The tool provides user-friendly error messages, including:

- **Rate Limiting**: When GitHub API rate limits are hit, shows both absolute reset time and relative countdown:
  ```
  🚨 GitHub API rate limit exceeded. Resets at: 2025-08-20 10:22:53 UTC (in 17min)
  ```
- **Network Issues**: Clear messages for download failures and connectivity problems
- **Installation Failures**: Detailed error output from failed commands or scripts

## Contributing

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the BSD-3 Clause License - see the [LICENSE](LICENSE) file for details.

## Advanced Examples

### Custom Installer with Download Function
```yaml
- name: "rustup"
  bin: "rustup"
  install_command: "{download(https://sh.rustup.rs, /tmp/rustup.sh)} && sh /tmp/rustup.sh -y --default-toolchain stable"
  update_command: "{bin_path} update"
```

### Script-based Installation
```yaml
- name: "my-app"
  bin: "my-app"
  repo: "user/my-app"  # For version checking
  script: "{app_path}/scripts/install-my-app.sh {bin_dir} {version} {os} {arch}"
```
