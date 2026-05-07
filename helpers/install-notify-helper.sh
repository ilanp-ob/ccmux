#!/usr/bin/env bash
# Build and install the ccmux-notify.app bundle.
# Run once; afterwards ccmux will use it automatically for notifications.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$SCRIPT_DIR/ccmux-notify/main.swift"
PLIST="$SCRIPT_DIR/ccmux-notify/Info.plist"

INSTALL_DIR="$HOME/.local/share/ccmux"
APP_DIR="$INSTALL_DIR/ccmux-notify.app"
MACOS_DIR="$APP_DIR/Contents/MacOS"
RESOURCES_DIR="$APP_DIR/Contents/Resources"

echo "Building ccmux-notify.app …"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

swiftc -O \
    -sdk "$(xcrun --show-sdk-path)" \
    -framework Cocoa \
    -framework UserNotifications \
    -o "$MACOS_DIR/ccmux-notify" \
    "$SRC"

cp "$PLIST" "$APP_DIR/Contents/Info.plist"

# Ad-hoc code sign so macOS allows the app to call UNUserNotificationCenter.
codesign --force --deep --sign - "$APP_DIR"

echo ""
echo "Installed → $APP_DIR"
echo ""
echo "Launching once to request notification permission…"
open "$APP_DIR"
echo "  ↳ If a permission dialog appeared, click Allow."
echo "  ↳ Then re-run 'ccmux notify' or wait for the next Claude completion."
