#!/usr/bin/env bash
# Builds a host-architecture Linux tarball (not a universal/static AppImage).
set -euo pipefail
[[ $(uname -s) == Linux ]] || { echo 'Build Linux packages on Linux.' >&2; exit 1; }
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BUILD=true
case "${1:-}" in
    --no-build) BUILD=false; shift ;;
    --help|-h) echo 'Usage: package-linux.sh [--no-build]'; exit 0 ;;
esac
(($# == 0)) || { echo 'Unexpected argument' >&2; exit 1; }
if $BUILD; then (cd -- "$ROOT" && cargo build --release --locked); fi
TARGET=${CARGO_TARGET_DIR:-"$ROOT/target"}
[[ $TARGET == /* ]] || TARGET="$ROOT/$TARGET"
BINARY="$TARGET/release/hfx"
[[ -x $BINARY ]] || { echo "Missing $BINARY" >&2; exit 1; }
VERSION=$("$BINARY" --version)
VERSION=${VERSION#hfx }
[[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+([a-zA-Z0-9.+-]*)$ ]] || { echo 'Invalid hfx version' >&2; exit 1; }
ARCH=$(uname -m)
[[ $ARCH =~ ^[a-zA-Z0-9_-]+$ ]] || exit 1
NAME="hfx-$VERSION-linux-$ARCH"
mkdir -p -- "$ROOT/dist"
STAGE=$(mktemp -d "$ROOT/dist/.hfx-package-XXXXXX")
trap 'rm -rf -- "$STAGE"' EXIT
mkdir -- "$STAGE/$NAME"
install -m 755 -- "$BINARY" "$STAGE/$NAME/hfx"
install -m 755 -- "$ROOT/scripts/install-linux.sh" "$STAGE/$NAME/install.sh"
install -m 755 -- "$ROOT/scripts/uninstall-linux.sh" "$STAGE/$NAME/uninstall.sh"
cp -- "$ROOT/LICENSE" "$STAGE/$NAME/LICENSE"
cp -- "$ROOT/assets/hfx.png" "$STAGE/$NAME/hfx-icon.png"
cat > "$STAGE/$NAME/README.txt" <<'EOF'
hfx — native AI coding workspace

Run ./install.sh --desktop-shortcut (no sudo).
Then find hfx in the application menu; optionally pin it to your dock.
Run ./uninstall.sh to remove it without deleting saved chats or credentials.
Run ./hfx directly for portable use (first launch creates a menu entry).

This is a native host-architecture Linux/glibc build. It still needs Wayland/X11,
OpenGL runtime libraries. Bubblewrap 0.12+ is only needed for the optional strict
command sandbox; trusted host commands are the default.
It is not a universal static binary, an AppImage, or a distro package.

Completion alerts: notify-send, plus paplay/pw-play/canberra-gtk-play for audio.
Settings → Appearance lets you mute either alert independently.
Agent access: Settings → Tools selects trusted host commands or strict confinement.
Trusted commands use your normal toolchains, package caches, Git/SSH, environment
and API keys, network and desktop access, without a command sandbox. They can
read credentials/change files outside the project. Use only with code you trust.
Commands default to a 30-minute timeout and preserve private workspace logs.
Files are not refused at 64 KiB: large UTF-8 files support context-aware pages.
Independent reads run up to four at a time; writes and reviewed actions stay ordered.
Hover a reply's elapsed time to distinguish model/network time from tool runtimes.
Web search/fetch work with every provider. DuckDuckGo needs no key; Brave Search
API and SearXNG are configurable alternatives. Web content is reference data.

Git attribution: Settings → Tools → Git attribution sets the co-author email.
The name is hfx; its default hfx@local.invalid address is an unlinked placeholder.
For a hosting-site co-author avatar, use an account you control, upload
hfx-icon.png to its profile, and use its verified/noreply email in that setting.
The commit message itself cannot embed an image. Existing history is not rewritten.

Set HFX_NO_DESKTOP_INTEGRATION=1 to disable first-launch launcher registration.
A Desktop shortcut may need “Allow Launching”; GNOME may need desktop-icons support.
EOF
OUT="$ROOT/dist/$NAME.tar.gz"
tar -C "$STAGE" -czf "$OUT" -- "$NAME"
(cd -- "$ROOT/dist" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
printf 'Package: %s\n' "$OUT"
