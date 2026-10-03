#!/usr/bin/env bash
# Shared by the macOS installer/uninstaller. Compatible with macOS Bash 3.2.

hfx_fail() { printf '%s\n' "$*" >&2; exit 1; }

hfx_macos_paths() {
    [[ $(uname -s) == Darwin ]] || hfx_fail 'This script is macOS-only.'
    [[ ${HOME:-} == /* ]] || hfx_fail 'HOME must be an absolute path.'
    [[ $APP_DIR == /* && $BIN_DIR == /* ]] || hfx_fail '--app-dir and --bin-dir must be absolute paths.'
    APP_DIR=${APP_DIR%/}
    BIN_DIR=${BIN_DIR%/}
    [[ -n $APP_DIR && -n $BIN_DIR && ! $APP_DIR =~ ^/+$ && ! $BIN_DIR =~ ^/+$ ]] || hfx_fail 'Do not install into a filesystem root.'
    case "$APP_DIR/$BIN_DIR/" in */../*) hfx_fail 'Use absolute paths without .. components.' ;; esac
    if [[ -d $APP_DIR && $(CDPATH= cd "$APP_DIR" && pwd -P) == / ]]; then hfx_fail 'Do not install into a filesystem root.'; fi
    if [[ -d $BIN_DIR && $(CDPATH= cd "$BIN_DIR" && pwd -P) == / ]]; then hfx_fail 'Do not install into a filesystem root.'; fi
    APP="$APP_DIR/hfx.app"
    CLI="$BIN_DIR/hfx"
    DESKTOP="$HOME/Desktop/hfx.app"
    MARKER="$APP/Contents/Resources/.hfx-macos-install"
    SIGNATURE=hfx-macos-install-v1
}

hfx_macos_owned_app() {
    [[ ! -L $APP && -d $APP && ! -L "$APP/Contents" && ! -L "$APP/Contents/Resources" && ! -L $MARKER && -f $MARKER ]] &&
        [[ $(cat "$MARKER") == "$SIGNATURE" ]]
}

hfx_macos_check_link() {
    local path=$1 target=$2
    if [[ -e $path || -L $path ]]; then
        [[ -L $path && $(readlink "$path") == "$target" ]] || hfx_fail "Refusing to replace unmanaged path: $path"
    fi
}

hfx_macos_remove_link() {
    local path=$1 target=$2
    if [[ -L $path && $(readlink "$path") == "$target" ]]; then
        rm "$path"
    elif [[ -e $path || -L $path ]]; then
        printf 'Preserved unrelated path: %s\n' "$path" >&2
    fi
}
