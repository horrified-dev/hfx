#!/usr/bin/env bash
# Install an unsigned, per-user .app and a CLI link. No sudo or Gatekeeper changes.
set -euo pipefail
SCRIPT_DIR=$(CDPATH= cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(CDPATH= cd "$SCRIPT_DIR/.." && pwd)
source "$SCRIPT_DIR/macos-common.sh"
APP_DIR="${HOME:?HOME is required}/Applications"
BIN_DIR="$HOME/.local/bin"
BINARY=''
BUILD=true
DESKTOP_SHORTCUT=false
while (($#)); do
    case "$1" in
        --desktop-shortcut) DESKTOP_SHORTCUT=true; shift ;;
        --no-build) BUILD=false; shift ;;
        --binary|--app-dir|--bin-dir)
            (($# >= 2)) || hfx_fail "Missing value for $1"
            case "$1" in
                --binary) BINARY=$2; BUILD=false ;;
                --app-dir) APP_DIR=$2 ;;
                --bin-dir) BIN_DIR=$2 ;;
            esac
            shift 2 ;;
        --help|-h)
            echo 'Usage: install-macos.sh [--desktop-shortcut] [--no-build] [--binary FILE] [--app-dir DIR] [--bin-dir DIR]'
            echo 'Run from a source checkout on macOS; builds for the current Rust target.'
            exit 0 ;;
        *) hfx_fail "Unknown option: $1" ;;
    esac
done
hfx_macos_paths
if [[ -e $APP || -L $APP ]]; then
    hfx_macos_owned_app || hfx_fail "Refusing to replace an unmanaged or symlinked app: $APP"
fi
hfx_macos_check_link "$CLI" "$APP/Contents/MacOS/hfx-bin"
if $DESKTOP_SHORTCUT; then hfx_macos_check_link "$DESKTOP" "$APP"; fi
if [[ -z $BINARY ]]; then
    if $BUILD; then
        command -v cargo >/dev/null 2>&1 || hfx_fail 'Install Rust 1.95+ and the Xcode Command Line Tools, or pass --binary FILE.'
        (cd "$ROOT" && cargo build --release --locked)
    fi
    TARGET=${CARGO_TARGET_DIR:-"$ROOT/target"}
    [[ $TARGET == /* ]] || TARGET="$ROOT/$TARGET"
    # CARGO_BUILD_TARGET, if set, places builds in a target-triple subdirectory.
    [[ -z ${CARGO_BUILD_TARGET:-} ]] || TARGET="$TARGET/$CARGO_BUILD_TARGET"
    BINARY="$TARGET/release/hfx"
fi
[[ -f $BINARY && -x $BINARY ]] || hfx_fail "No executable at $BINARY. Build first, or pass --binary FILE."
[[ -f "$ROOT/assets/hfx.icns" ]] || hfx_fail 'Missing assets/hfx.icns; use a complete checkout.'
VERSION=$(awk -F '"' '/^version[[:space:]]*=/ {print $2; exit}' "$ROOT/Cargo.toml")
VERSION=${VERSION%%-*}
[[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || hfx_fail 'Cannot determine the app bundle version from Cargo.toml.'
mkdir -p "$APP_DIR" "$BIN_DIR"
if $DESKTOP_SHORTCUT; then mkdir -p "$HOME/Desktop"; fi
STAGE=$(mktemp -d "$APP_DIR/.hfx-install-XXXXXX")
PUBLISHED=false
BACKUP=false
CLI_CREATED=false
DESKTOP_CREATED=false
cleanup() {
    local status=$?
    if ((status != 0)); then
        if $CLI_CREATED; then hfx_macos_remove_link "$CLI" "$APP/Contents/MacOS/hfx-bin"; fi
        if $DESKTOP_CREATED; then hfx_macos_remove_link "$DESKTOP" "$APP"; fi
        if $PUBLISHED; then rm -rf "$APP"; fi
        if $BACKUP; then mv "$STAGE/previous.app" "$APP"; fi
    fi
    rm -rf "$STAGE"
}
trap cleanup EXIT
BUNDLE="$STAGE/hfx.app"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
install -m 755 "$BINARY" "$BUNDLE/Contents/MacOS/hfx-bin"
install -m 644 "$ROOT/assets/hfx.icns" "$BUNDLE/Contents/Resources/hfx.icns"
install -m 644 "$ROOT/LICENSE" "$BUNDLE/Contents/Resources/LICENSE"
printf '%s\n' "$SIGNATURE" > "$BUNDLE/Contents/Resources/.hfx-macos-install"
cat > "$BUNDLE/Contents/MacOS/hfx" <<'LAUNCHER'
#!/bin/sh
# Finder launches a fresh profile in a small starter workspace, not / or $HOME.
CONTENTS=$(CDPATH= cd "$(dirname "$0")/.." && pwd) || exit 1
WORKSPACE="${HOME:?HOME is required}/Library/Application Support/hfx/workspace"
mkdir -p "$WORKSPACE" || exit 1
cd "$WORKSPACE" || exit 1
exec "$CONTENTS/MacOS/hfx-bin" "$@"
LAUNCHER
chmod 755 "$BUNDLE/Contents/MacOS/hfx"
cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.horrified.hfx</string>
  <key>CFBundleName</key><string>hfx</string>
  <key>CFBundleDisplayName</key><string>hfx</string>
  <key>CFBundleExecutable</key><string>hfx</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>hfx.icns</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict></plist>
PLIST
# Stage on the same filesystem. Restore the previous bundle if publication fails.
if [[ -d $APP ]]; then mv "$APP" "$STAGE/previous.app"; BACKUP=true; fi
mv "$BUNDLE" "$APP"
PUBLISHED=true
# Existing owned links already point to the new bundle and need not be replaced.
if [[ ! -L $CLI ]]; then ln -s "$APP/Contents/MacOS/hfx-bin" "$CLI"; CLI_CREATED=true; fi
if $DESKTOP_SHORTCUT && [[ ! -L $DESKTOP ]]; then ln -s "$APP" "$DESKTOP"; DESKTOP_CREATED=true; fi
printf '\nInstalled hfx: %s\nTerminal command: %s\n' "$APP" "$CLI"
echo 'Open hfx.app in Finder, then choose Keep in Dock if desired. Quit/restart hfx after updates.'
echo 'The local app is not Developer-ID signed or notarized; macOS security checks remain enabled.'
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) printf 'For terminal launching, add %s to PATH.\n' "$BIN_DIR" ;; esac
