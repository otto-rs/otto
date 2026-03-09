#!/bin/sh
# Install otto - https://github.com/otto-rs/otto
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/otto-rs/otto/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/otto-rs/otto/main/install.sh | bash -s -- --to ~/bin
#   curl -fsSL https://raw.githubusercontent.com/otto-rs/otto/main/install.sh | bash -s -- --version v1.0.0

set -eu

REPO="otto-rs/otto"
BINARY="otto"
DEFAULT_INSTALL_DIR="/usr/local/bin"

main() {
    install_dir="$DEFAULT_INSTALL_DIR"
    version="latest"
    need_sudo=""

    while [ $# -gt 0 ]; do
        case "$1" in
            --to)
                install_dir="$2"
                shift 2
                ;;
            --version)
                version="$2"
                shift 2
                ;;
            *)
                err "Unknown option: $1"
                ;;
        esac
    done

    platform="$(detect_platform)"
    arch="$(detect_arch)"
    suffix="$(get_suffix "$platform" "$arch")"

    if [ "$version" = "latest" ]; then
        tag="$(get_latest_tag)"
    else
        tag="$version"
    fi

    say "Installing ${BINARY} ${tag} (${suffix})"

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    tarball="otto-${tag}-${suffix}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${tag}/${tarball}"
    checksum_url="${url}.sha256"

    say "Downloading ${url}"
    download "$url" "${tmpdir}/${tarball}"
    download "$checksum_url" "${tmpdir}/${tarball}.sha256"

    say "Verifying checksum"
    verify_checksum "${tmpdir}/${tarball}" "${tmpdir}/${tarball}.sha256"

    say "Extracting binary"
    tar -xzf "${tmpdir}/${tarball}" -C "${tmpdir}"

    # Determine if we need sudo
    if [ ! -d "$install_dir" ]; then
        if ! mkdir -p "$install_dir" 2>/dev/null; then
            need_sudo="yes"
        fi
    elif [ ! -w "$install_dir" ]; then
        need_sudo="yes"
    fi

    if [ -n "$need_sudo" ]; then
        if ! command -v sudo >/dev/null 2>&1; then
            err "Install directory '${install_dir}' is not writable and sudo is not available. Run as root or use --to <dir>."
        fi
        say "Installing to ${install_dir} (requires sudo)"
        sudo mkdir -p "$install_dir"
        sudo install -m 755 "${tmpdir}/${BINARY}" "${install_dir}/${BINARY}"
    else
        mkdir -p "$install_dir"
        install -m 755 "${tmpdir}/${BINARY}" "${install_dir}/${BINARY}"
    fi

    say "Installed ${BINARY} to ${install_dir}/${BINARY}"
    "${install_dir}/${BINARY}" --version 2>/dev/null || true
}

detect_platform() {
    os="$(uname -s)"
    case "$os" in
        Linux)  echo "linux" ;;
        Darwin) echo "macos" ;;
        *)      err "Unsupported platform: ${os}" ;;
    esac
}

detect_arch() {
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64)   echo "x86_64" ;;
        aarch64|arm64)  echo "arm64" ;;
        *)              err "Unsupported architecture: ${arch}" ;;
    esac
}

get_suffix() {
    platform="$1"
    arch="$2"
    case "${platform}-${arch}" in
        linux-x86_64)  echo "linux-amd64" ;;
        linux-arm64)   echo "linux-arm64" ;;
        macos-x86_64)  echo "macos-x86_64" ;;
        macos-arm64)   echo "macos-arm64" ;;
        *)             err "Unsupported platform/arch: ${platform}/${arch}" ;;
    esac
}

get_latest_tag() {
    url="https://api.github.com/repos/${REPO}/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        tag="$(curl -fsSL "$url" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')"
    elif command -v wget >/dev/null 2>&1; then
        tag="$(wget -qO- "$url" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')"
    else
        err "curl or wget is required"
    fi
    if [ -z "$tag" ]; then
        err "Could not determine latest release tag"
    fi
    echo "$tag"
}

download() {
    url="$1"
    dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$dest" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        err "curl or wget is required"
    fi
}

verify_checksum() {
    file="$1"
    checksum_file="$2"
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$(dirname "$file")" && sha256sum -c "$(basename "$checksum_file")" --quiet)
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$(dirname "$file")" && shasum -a 256 -c "$(basename "$checksum_file")" --quiet)
    else
        say "Warning: cannot verify checksum (sha256sum/shasum not found)"
    fi
}

say() {
    printf "otto-install: %s\n" "$1"
}

err() {
    say "ERROR: $1" >&2
    exit 1
}

main "$@"
