#!/usr/bin/env bash
# Remove our app bundle and matching links, never chats or OAuth credentials.
set -euo pipefail
SCRIPT_DIR=$(CDPATH= cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "$SCRIPT_DIR/macos-common.sh"
APP_DIR="${HOME:?HOME is required}/Applications"
BIN_DIR="$HOME/.local/bin"
while (($#)); do
    case "$1" in
        --app-dir|--bin-dir)
            (($# >= 2)) || hfx_fail "Missing value for $1"
            if [[ $1 == --app-dir ]]; then APP_DIR=$2; else BIN_DIR=$2; fi
            shift 2 ;;
        --help|-h) echo 'Usage: uninstall-macos.sh [--app-dir DIR] [--bin-dir DIR]'; exit 0 ;;
        *) hfx_fail "Unknown option: $1" ;;
    esac
done
hfx_macos_paths
hfx_macos_owned_app || hfx_fail "No script-managed hfx app at $APP; nothing was removed."
hfx_macos_remove_link "$CLI" "$APP/Contents/MacOS/hfx-bin"
hfx_macos_remove_link "$DESKTOP" "$APP"
rm -rf "$APP"
echo 'hfx uninstalled. Saved chats, preferences, credentials, and workspace files were preserved.'
