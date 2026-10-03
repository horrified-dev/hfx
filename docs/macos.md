# macOS installation

[Back to README](../README.md)

## Requirements

- macOS 11 or later, with a desktop session and compatible OpenGL support.
- For source builds: **Rust 1.95+** and Apple's **Xcode Command Line Tools** (`xcode-select --install`). Use the Rust toolchain for the Mac's architecture (Apple Silicon or Intel).
- Run the installer from a complete checkout. It does not download an executable or install system packages.

## Install and update

```bash
./scripts/install-macos.sh --desktop-shortcut
```

The script builds a release binary and installs **for the current user**, without `sudo`:

- App bundle: `~/Applications/hfx.app`, including the hfx `.icns` icon and license.
- Terminal command: `~/.local/bin/hfx`, a symlink to the bundled executable. Add `~/.local/bin` to your shell's `PATH` if it is not already present.
- Optional Desktop shortcut: `~/Desktop/hfx.app`, a symlink to the bundle. Omit `--desktop-shortcut` if you do not want it.

Open the app in Finder, then use **Options → Keep in Dock** to pin it. Finder launches a fresh profile in `~/Library/Application Support/hfx/workspace`; terminal launches use their current directory. Existing saved projects/chats are not reset. Finder does not source your shell startup files: launch `hfx` from a terminal when you need that shell's custom PATH or Git/SSH environment.

Quit hfx, rerun the installer to update, and reopen the app. The bundle is staged beside its destination; a failed publication restores the previous bundle. Unrelated apps, CLI files, and Desktop entries are not overwritten. A previously created, matching Desktop shortcut remains when the flag is omitted on update.

### Existing builds and custom paths

```bash
./scripts/install-macos.sh --no-build
./scripts/install-macos.sh --binary /absolute/path/to/hfx
./scripts/install-macos.sh --app-dir "$HOME/Apps" --bin-dir "$HOME/bin"
```

`--binary` skips building; supply a compatible **macOS executable**, not a Linux or Windows build. `--no-build` uses the existing `target/release/hfx` (or `CARGO_TARGET_DIR`, with the `CARGO_BUILD_TARGET` subdirectory if set). For other custom Cargo target configurations, use `--binary` explicitly. The installer builds one target, not a universal Apple Silicon/Intel application.

### macOS security prompts

The locally assembled bundle is **not Developer-ID signed or notarized**. A downloaded executable or checkout may trigger Gatekeeper. Only open code you trust; follow macOS's explicit **Open Anyway** flow in **System Settings → Privacy & Security** if appropriate. The installer does not remove quarantine attributes, bypass Gatekeeper, or change system security settings. Managed-device policy can still prevent execution.

## Uninstall without deleting data

Quit hfx, then run:

```bash
./scripts/uninstall-macos.sh
# If custom locations were used, supply the same paths:
./scripts/uninstall-macos.sh --app-dir "$HOME/Apps" --bin-dir "$HOME/bin"
```

Only a script-managed `hfx.app` bundle and still-matching CLI/Desktop links are removed. Unrelated replacement links/files are preserved. Chats, preferences, the Codex OAuth cache, and starter workspace under `~/Library/Application Support/hfx` are **not deleted**.

## Platform limitations

Native completion notifications/sound and strict command confinement are currently Linux-only. The in-app completion toast and trusted host commands work on macOS. See [Tools & security](tools.md) before granting command access.
