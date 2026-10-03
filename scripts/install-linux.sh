#!/usr/bin/env bash
# Per-user source or portable-bundle installer; never uses sudo.
set -euo pipefail
[[ $(uname -s) == Linux ]] || { echo 'This installer is Linux-only.' >&2; exit 1; }
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ -f "$SCRIPT_DIR/../Cargo.toml" ]]; then ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd); else ROOT=$SCRIPT_DIR; fi
BIN_DIR=${XDG_BIN_HOME:-"${HOME:?HOME is required}/.local/bin"}
BUILD=true
BINARY=''
DESKTOP=false
while (($#)); do
    case "$1" in
        --desktop-shortcut) DESKTOP=true; shift ;;
        --no-build) BUILD=false; shift ;;
        --binary|--bin-dir)
            (($# >= 2)) || { echo "Missing value for $1" >&2; exit 1; }
            if [[ $1 == --binary ]]; then BINARY=$2; BUILD=false; else BIN_DIR=$2; fi
            shift 2 ;;
        --help|-h)
            echo 'Usage: install-linux.sh [--desktop-shortcut] [--no-build] [--binary FILE] [--bin-dir DIR]'
            echo 'Builds from source, or installs the hfx executable supplied in a portable bundle.'
            exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done
[[ $BIN_DIR == /* ]] || { echo '--bin-dir / XDG_BIN_HOME must be absolute.' >&2; exit 1; }
if [[ -z $BINARY ]]; then
    if [[ -f "$ROOT/Cargo.toml" ]]; then
        if $BUILD; then (cd -- "$ROOT" && cargo build --release --locked); fi
        TARGET=${CARGO_TARGET_DIR:-"$ROOT/target"}
        [[ $TARGET == /* ]] || TARGET="$ROOT/$TARGET"
        BINARY="$TARGET/release/hfx"
    else
        BINARY="$ROOT/hfx"
    fi
fi
[[ -f $BINARY && -x $BINARY ]] || { echo "No executable at $BINARY. Build first, or pass --binary FILE." >&2; exit 1; }
# Resolve before any copy so source paths with spaces or a relative cwd work.
BINARY=$(realpath -- "$BINARY")
DEST="$BIN_DIR/hfx"
MARKER="$BIN_DIR/.hfx-user-install"
if [[ -L $DEST || -L $MARKER || ( -e $DEST && ! -f $DEST ) ]]; then
    echo 'Refusing symlinks or non-files in a script-managed installation.' >&2
    exit 1
fi
SIGNATURE=hfx-user-install-v1
OWNED=false
[[ -f $MARKER && $(cat -- "$MARKER") == "$SIGNATURE" ]] && OWNED=true
if [[ -e $DEST || -L $DEST ]] && ! $OWNED; then
    echo "Refusing to overwrite $DEST: it wasn't installed by this script." >&2
    exit 1
fi
mkdir -p -- "$BIN_DIR"
TEMP=$(mktemp "$BIN_DIR/.hfx-install-XXXXXX")
BACKUP=''
cleanup() { rm -f -- "$TEMP"; [[ -z $BACKUP ]] || rm -f -- "$BACKUP"; }
trap cleanup EXIT
if [[ -f $DEST ]]; then BACKUP=$(mktemp "$BIN_DIR/.hfx-backup-XXXXXX"); cp -p -- "$DEST" "$BACKUP"; fi
install -m 755 -- "$BINARY" "$TEMP"
# Rename permits updates while the old executable is still running.
mv -fT -- "$TEMP" "$DEST"
printf '%s\n' "$SIGNATURE" > "$MARKER"
chmod 600 -- "$MARKER"
ARGS=(--install-desktop)
$DESKTOP && ARGS+=(--desktop-shortcut)
if ! HFX_NO_DESKTOP_INTEGRATION=1 "$DEST" "${ARGS[@]}"; then
    if [[ -n $BACKUP ]]; then mv -fT -- "$BACKUP" "$DEST"; BACKUP=''; else rm -f -- "$DEST"; fi
    $OWNED || rm -f -- "$MARKER"
    echo 'Desktop registration failed; the previous executable was restored.' >&2
    exit 1
fi
printf '\nInstalled hfx: %s\nFind “hfx” in your application menu and pin it to your dock.\n' "$DEST"
if $DESKTOP; then echo 'Your desktop may require right-click → Allow Launching (and a desktop-icons extension on GNOME).'; fi
if ! command -v notify-send >/dev/null 2>&1; then echo 'Optional completion notifications: install notify-send (Debian/Ubuntu: libnotify-bin).'; fi
if ! command -v paplay >/dev/null 2>&1 && ! command -v pw-play >/dev/null 2>&1 && ! command -v canberra-gtk-play >/dev/null 2>&1; then echo 'Optional completion sound: install paplay, pw-play, or canberra-gtk-play.'; fi
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) echo "For terminal launching, add $BIN_DIR to PATH." ;; esac
