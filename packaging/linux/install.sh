#!/bin/sh
# Installs Parameter Save States for the current user.
#
#   ./install.sh            install into ~/.local/share/parametersavestates
#   ./install.sh --remove   uninstall
#
# The SteamVR manifest is registered automatically the first time the app
# connects to SteamVR.
set -eu

APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/parametersavestates/app"
BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"

if [ "${1:-}" = "--remove" ]; then
    rm -rf "$APP_DIR"
    rm -f "$BIN_DIR/ParameterSaveStates" \
          "$DESKTOP_DIR/parametersavestates.desktop" \
          "$ICON_DIR/parametersavestates.png"
    echo "Removed. Your profiles are kept in ${XDG_DATA_HOME:-$HOME/.local/share}/parametersavestates"
    exit 0
fi

mkdir -p "$APP_DIR" "$BIN_DIR" "$DESKTOP_DIR" "$ICON_DIR"
cp "$SRC_DIR/ParameterSaveStates" "$SRC_DIR/libopenvr_api.so" \
   "$SRC_DIR/app.vrmanifest" "$SRC_DIR/icon.png" "$APP_DIR/"
chmod +x "$APP_DIR/ParameterSaveStates"
ln -sf "$APP_DIR/ParameterSaveStates" "$BIN_DIR/ParameterSaveStates"
cp "$SRC_DIR/icon.png" "$ICON_DIR/parametersavestates.png"
sed "s|^Exec=.*|Exec=$APP_DIR/ParameterSaveStates|" \
    "$SRC_DIR/parametersavestates.desktop" > "$DESKTOP_DIR/parametersavestates.desktop"

echo "Installed to $APP_DIR"
echo "Start it from your application menu or run: $BIN_DIR/ParameterSaveStates"
