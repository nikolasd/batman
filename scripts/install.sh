#!/bin/bash
set -e

# BATMAN installer
# Installs both the batcave runtime and the OMP extension to ~/.batman
# No root privileges required.

INSTALL_DIR="$HOME/.batman"
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"

# Clean up old installation if exists
if [ -d "$INSTALL_DIR" ]; then
  echo "Found existing BATMAN installation at $INSTALL_DIR"
  echo "Removing old installation..."
  rm -rf "$INSTALL_DIR"
fi

# Create directories
mkdir -p "$BIN_DIR" "$LIB_DIR"

# Detect platform
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

echo "Installing BATMAN v${VERSION} to $INSTALL_DIR..."
echo "Platform: $PLATFORM/$ARCH"

# Download the binary
echo "Downloading batcave binary from $GITHUB_URL..."
if command -v curl &> /dev/null; then
  curl -L -o "$BIN_DIR/batcave" "$GITHUB_URL"
elif command -v wget &> /dev/null; then
  wget -O "$BIN_DIR/batcave" "$GITHUB_URL"
else
  echo "Error: Neither curl nor wget found. Please install one and retry." >&2
  exit 1
fi

chmod +x "$BIN_DIR/batcave"

# Install the OMP extension via bun
echo "Installing OMP extension..."
if command -v bun &> /dev/null; then
  cd "$LIB_DIR"
  bun init -y 2>/dev/null || true
  bun add @satori/batman 2>/dev/null || {
    echo "Warning: Failed to install @satori/batman via bun. You may need to install it manually."
    echo "Run: bun add @satori/batman --cwd $LIB_DIR"
  }
  cd - > /dev/null
else
  echo "Warning: bun not found. You will need to install the OMP extension manually."
  echo "Run: bun add @satori/batman --cwd $LIB_DIR"
fi

# Add to PATH (create a symlink in /usr/local/bin if possible, otherwise warn)
if [ -w "/usr/local/bin" ] || [ "$(id -u)" -eq 0 ]; then
  ln -sf "$BIN_DIR/batcave" /usr/local/bin/batcave 2>/dev/null || true
else
  echo "Warning: Cannot create symlink in /usr/local/bin without root privileges."
  echo "Add $BIN_DIR to your PATH manually, or run this script with sudo."
  echo "Alternatively, add the following to your ~/.bashrc or ~/.zshrc:"
  echo 'export PATH="$HOME/.batman/bin:$PATH"'
fi

echo ""
echo "BATMAN v${VERSION} installed successfully!"
echo ""
echo "Installation directory: $INSTALL_DIR"
echo "  Binary: $BIN_DIR/batcave"
echo "  Extension: $LIB_DIR/node_modules/@satori/batman"
echo ""
echo "To use BATMAN:"
echo "  1. Ensure ~/.batman/bin is in your PATH (add to ~/.bashrc or ~/.zshrc if needed)"
echo "  2. Run: batcave serve --state-dir \$HOME/.batman/state --repo \$PWD"
echo "  3. Or through OMP: omp --extension ~/.batman/lib/node_modules/@satori/batman/dist/index.js"
echo ""
echo "To uninstall:"
echo "  rm -rf $INSTALL_DIR"
echo "  rm -f /usr/local/bin/batcave 2>/dev/null || true"
echo ""
