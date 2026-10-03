#!/usr/bin/env bash
# Removes only our binary/desktop integration; never chats or login credentials.
set -euo pipefail
[[ $(uname -s) == Linux ]] || { echo 'This uninstaller is Linux-only.' >&2; exit 1; }
BIN_DIR=${XDG_BIN_HOME:-"${HOME:?HOME is required}/.local/bin"}
while (($#)); do
    case "$1" in
        --bin-dir) (($# >= 2)) || { echo 'Missing --bin-dir value' >&2; exit 1; }; BIN_DIR=$2; shift 2 ;;
        --help|-h) echo 'Usage: uninstall-linux.sh [--bin-dir DIR]'; exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done
[[ $BIN_DIR == /* ]] || { echo '--bin-dir / XDG_BIN_HOME must be absolute.' >&2; exit 1; }
DEST="$BIN_DIR/hfx"
MARKER="$BIN_DIR/.hfx-user-install"
if [[ -L $DEST || -L $MARKER || ( -e $DEST && ! -f $DEST ) ]]; then
    echo 'Refusing symlinks or non-files in a script-managed installation.' >&2
    exit 1
fi
if [[ ! -f $MARKER || $(cat -- "$MARKER") != hfx-user-install-v1 ]]; then
    echo "No script-managed hfx installation in $BIN_DIR; nothing was removed." >&2
    exit 1
fi
if [[ -x $DEST ]]; then HFX_NO_DESKTOP_INTEGRATION=1 "$DEST" --uninstall-desktop; fi
rm -f -- "$DEST" "$MARKER"
echo 'hfx uninstalled. Saved chats, preferences, credentials, and workspace files were preserved.'
