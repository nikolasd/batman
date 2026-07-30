#!/bin/bash
set -e

# BATMAN installer
# Downloads and installs the batcave binary for the current platform

PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$PLATFORM" in
  darwin)
    case "$ARCH" in
      arm64) BINARY="batcave-darwin-arm64" ;;
      x86_64) BINARY="batcave-darwin-x64" ;;
      *) echo "Error: Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  linux)
    case "$ARCH" in
      aarch64) BINARY="batcave-linux-arm64-gnu" ;;
      x86_64) BINARY="batcave-linux-x64-gnu" ;;
      *) echo "Error: Unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *) echo "Error: Unsupported platform: $PLATFORM" >&2; exit 1 ;;
esac

VERSION="0.1.0"
GITHUB_URL="https://github.com/nikolasd/batman/releases/download/v${VERSION}/${BINARY}"

echo "Installing BATMAN v${VERSION} for ${PLATFORM}/${ARCH}..."
echo "Downloading from: ${GITHUB_URL}"

# Download the binary
if command -v curl &> /dev/null; then
  curl -L -o /tmp/batcave "$GITHUB_URL"
elif command -v wget &> /dev/null; then
  wget -O /tmp/batcave "$GITHUB_URL"
else
  echo "Error: Neither curl nor wget found. Please install one and retry." >&2
  exit 1
fi

# Make it executable
chmod +x /tmp/batcave

# Install to /usr/local/bin (requires sudo)
if [ "$(id -u)" -eq 0 ]; then
  mv /tmp/batcave /usr/local/bin/batcave
else
  if sudo -n true 2>/dev/null; then
    sudo mv /tmp/batcave /usr/local/bin/batcave
  else
    echo "Error: Installation requires root privileges. Run with sudo." >&2
    exit 1
  fi
fi

echo "BATMAN v${VERSION} installed to /usr/local/bin/batcave"
echo "Run 'batcave --version' to verify."
