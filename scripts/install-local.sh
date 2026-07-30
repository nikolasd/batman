#!/bin/bash
set -e

# BATMAN local installer
# Installs from local repository files (no GitHub Releases required)
# Works immediately for macOS ARM. Other platforms require building first.
# Must be run from the repository root (or this script will cd there automatically).

# cd to the directory containing this script (the repo root)
cd "$(dirname "$0")/.." 2>/dev/null || true

INSTALL_DIR="$HOME/.batman"
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"

# Detect platform
PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Find the local binary
case "$PLATFORM/$ARCH" in
  "darwin/arm64")
    LOCAL_BINARY="packages/batman-darwin-arm64/bin/batcave"
    ;;
  "darwin/x86_64")
    LOCAL_BINARY="packages/batman-darwin-x64/bin/batcave"
    ;;
  "linux/x86_64")
    LOCAL_BINARY="packages/batman-linux-x64-gnu/bin/batcave"
    ;;
  "linux/aarch64")
    LOCAL_BINARY="packages/batman-linux-arm64-gnu/bin/batcave"
    ;;
  *)
    echo "Error: No pre-built binary found for $PLATFORM/$ARCH" >&2
    echo "You must build from source first:" >&2
    echo "  cargo build -p batman-runtime" >&2
    echo "Then copy the binary to the appropriate packages/batman-*/bin/ directory." >&2
    exit 1
    ;;
esac

if [ ! -f "$LOCAL_BINARY" ]; then
  echo "Error: Local binary not found at $LOCAL_BINARY" >&2
  echo "You must build from source first:" >&2
  echo "  cargo build -p batman-runtime" >&2
  exit 1
fi

# Clean up old installation if exists
if [ -d "$INSTALL_DIR" ]; then
  echo "Found existing BATMAN installation at $INSTALL_DIR"
  echo "Removing old installation..."
  rm -rf "$INSTALL_DIR"
fi

# Create directories
mkdir -p "$BIN_DIR" "$LIB_DIR"

VERSION=$(grep '"version"' packages/extension/package.json | head -1 | sed 's/.*: "\(.*\)".*/\1/')

echo "Installing BATMAN v${VERSION} to $INSTALL_DIR..."
echo "Platform: $PLATFORM/$ARCH"
echo "Using local binary: $LOCAL_BINARY"

# Copy the binary
cp "$LOCAL_BINARY" "$BIN_DIR/batcave"
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
