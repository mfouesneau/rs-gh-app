#!/bin/sh
# shellcheck shell=dash
# shellcheck disable=SC2039  # local is non-POSIX
# This runs on Unix shells like bash/dash/ksh/zsh. It uses the common `local`
# extension. Note: Most shells limit `local` to 1 var per line, contra bash.

# Some versions of ksh have no `local` keyword. Alias it to `typeset`, but
# beware this makes variables global with f()-style function syntax in ksh93.
# mksh has this alias by default.
has_local() {
   # shellcheck disable=SC2034  # deliberately unused
   local _has_local
}

has_local 2>/dev/null || alias local=typeset

set -u

APP_NAME="rs-gh-app"
APP_REPO="mfouesneau/rs-gh-app"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_info() {
    printf "${BLUE}ℹ️  %s${NC}\n" "$1"
}

print_success() {
    printf "${GREEN}✅ %s${NC}\n" "$1"
}

print_warning() {
    printf "${YELLOW}⚠️  %s${NC}\n" "$1"
}

print_error() {
    printf "${RED}❌ %s${NC}\n" "$1"
}

# Function to detect OS
detect_os() {
    local os
    case "$(uname -s)" in
        Linux*)               os="linux";;
        Darwin*)              os="darwin";;
        CYGWIN*|MINGW*|MSYS*) os="windows";;
        *)                    os="unknown";;
    esac
    echo "$os"
}

# Function to detect architecture
detect_arch() {
    local arch
    case "$(uname -m)" in
        x86_64|amd64)   arch="x86_64";;
        aarch64|arm64)  arch="aarch64";;
        i386|i686)      arch="i686";;
        *)              arch="unknown";;
    esac
    echo "$arch"
}

# Function to get the latest release tag from GitHub
get_latest_release() {
    local repo="$1"
    local tag

    # First try using curl with GitHub API
    if command -v curl >/dev/null 2>&1; then
        tag=$(curl -s "https://api.github.com/repos/$repo/releases/latest" | \
              grep '"tag_name":' | \
              sed -E 's/.*"([^"]+)".*/\1/')
    # Fallback to wget if curl is not available
    elif command -v wget >/dev/null 2>&1; then
        tag=$(wget -qO- "https://api.github.com/repos/$repo/releases/latest" | \
              grep '"tag_name":' | \
              sed -E 's/.*"([^"]+)".*/\1/')
    else
        print_error "Neither curl nor wget is available"
        return 1
    fi

    if [ -z "$tag" ]; then
        print_error "Failed to get latest release tag"
        return 1
    fi

    echo "$tag"
}

# Function to construct download URL
construct_download_url() {
    local repo="$1"
    local tag="$2"
    local os="$3"
    local arch="$4"
    local filename

    # Construct filename based on your release naming convention
    # Based on your README, it looks like: rs-gh-app-x86_64-apple-darwin.tar.gz
    case "$os" in
        linux)
            case "$arch" in
                x86_64)   filename="${APP_NAME}-x86_64-unknown-linux-musl.tar.gz";;
                aarch64)  filename="${APP_NAME}-aarch64-unknown-linux-musl.tar.gz";;
                *)        print_error "Unsupported architecture: $arch"; return 1;;
            esac
            ;;
        darwin)
            case "$arch" in
                x86_64)   filename="${APP_NAME}-x86_64-apple-darwin.tar.gz";;
                aarch64)  filename="${APP_NAME}-aarch64-apple-darwin.tar.gz";;
                *)        print_error "Unsupported architecture: $arch"; return 1;;
            esac
            ;;
        windows)
            case "$arch" in
                x86_64)   filename="${APP_NAME}-x86_64-pc-windows-msvc.zip";;
                aarch64)  filename="${APP_NAME}-aarch64-pc-windows-msvc.zip";;
                *)        print_error "Unsupported architecture: $arch"; return 1;;
            esac
            ;;
        *)
            print_error "Unsupported OS: $os"
            return 1
            ;;
    esac

    echo "https://github.com/$repo/releases/download/$tag/$filename"
}

# Function to download file
download_file() {
    local url="$1"
    local output_path="$2"

    print_info "Downloading from $url"

    if command -v curl >/dev/null 2>&1; then
        curl -L -o "$output_path" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$output_path" "$url"
    else
        print_error "Neither curl nor wget is available"
        return 1
    fi
}

# Function to extract archive
extract_archive() {
    local archive_path="$1"
    local extract_dir="$2"
    local filename
    filename=$(basename "$archive_path")

    case "$filename" in
        *.tar.gz|*.tgz)
            if command -v tar >/dev/null 2>&1; then
                tar -xzf "$archive_path" -C "$extract_dir"
            else
                print_error "tar is not available"
                return 1
            fi
            ;;
        *.zip)
            if command -v unzip >/dev/null 2>&1; then
                unzip -q "$archive_path" -d "$extract_dir"
            else
                print_error "unzip is not available"
                return 1
            fi
            ;;
        *)
            print_error "Unsupported archive format: $filename"
            return 1
            ;;
    esac
}

# Function to find binary in extracted files
find_binary() {
    local search_dir="$1"
    local binary_name="$2"

    # Look for the binary recursively
    find "$search_dir" -name "$binary_name" -type f 2>/dev/null | head -1
}

# Main installation function
install_app() {
    local os
    local arch
    local tag
    local download_url
    local bin_dir
    local temp_dir
    local archive_path
    local extracted_binary
    local final_binary_path

    print_info "Starting installation of $APP_NAME"

    # Detect system
    os=$(detect_os)
    arch=$(detect_arch)

    if [ "$os" = "unknown" ] || [ "$arch" = "unknown" ]; then
        print_error "Unsupported system: $os/$arch"
        return 1
    fi

    print_info "Detected system: $os/$arch"

    # Determine installation directory
    if [ -n "${BIN_DIR:-}" ]; then
        bin_dir="$BIN_DIR"
    else
        bin_dir="$HOME/.local/bin"
    fi

    # Create bin directory if it doesn't exist
    if [ ! -d "$bin_dir" ]; then
        print_info "Creating directory: $bin_dir"
        mkdir -p "$bin_dir" || {
            print_error "Failed to create directory: $bin_dir"
            return 1
        }
    fi

    print_info "Installation directory: $bin_dir"

    # Get latest release
    tag=$(get_latest_release "$APP_REPO")
    if [ $? -ne 0 ]; then
        return 1
    fi

    print_info "Latest release: $tag"

    # Construct download URL
    download_url=$(construct_download_url "$APP_REPO" "$tag" "$os" "$arch")
    if [ $? -ne 0 ]; then
        return 1
    fi

    # Create temporary directory
    temp_dir=$(mktemp -d)
    archive_path="$temp_dir/$(basename "$download_url")"

    # Download the archive
    if ! download_file "$download_url" "$archive_path"; then
        print_error "Failed to download $download_url"
        rm -rf "$temp_dir"
        return 1
    fi

    print_success "Download completed"

    # Extract the archive
    print_info "Extracting archive into $temp_dir ..."
    if ! extract_archive "$archive_path" "$temp_dir"; then
        print_error "Failed to extract archive"
        rm -rf "$temp_dir"
        return 1
    fi

    # Find the binary
    extracted_binary=$(find_binary "$temp_dir" "$APP_NAME")
    if [ -z "$extracted_binary" ]; then
        print_error "Binary '$APP_NAME' not found in extracted files"
        rm -rf "$temp_dir"
        return 1
    fi

    print_info "Found binary: $extracted_binary"

    # Install the binary
    final_binary_path="$bin_dir/$APP_NAME"
    if ! cp "$extracted_binary" "$final_binary_path"; then
        print_error "Failed to copy binary to $final_binary_path"
        rm -rf "$temp_dir"
        return 1
    fi

    # Make sure it's executable
    chmod +x "$final_binary_path"

    # Clean up
    rm -rf "$temp_dir"

    print_success "$APP_NAME $tag installed successfully to $final_binary_path"

    # Check if bin_dir is in PATH
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *)
            print_warning "$bin_dir is not in your PATH"
            print_info "Add the following line to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
            print_info "export PATH=\"$bin_dir:\$PATH\""
            ;;
    esac

    print_info "You can now run: $APP_NAME --help"
}

# Run the installation
install_app
