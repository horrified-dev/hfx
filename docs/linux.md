# Linux installation and desktop integration

[README](../README.md) · [Providers](providers.md) · [Usage](usage.md) · [Tools & security](tools.md) · [Linux](linux.md) · [Development](development.md)

Run the commands below from the repository root. Source builds require Rust 1.95 or later, a desktop session, and compatible Wayland/X11 and OpenGL libraries. Build dependencies may include a C compiler, `pkg-config`, and development packages for X11, Wayland, and xkbcommon. The renderer uses eframe’s Glow backend.

## Linux installation and desktop launcher

From this checkout:

```bash
./scripts/install-linux.sh --desktop-shortcut
```

This builds the release binary and installs it **for your user only**, without `sudo`:

- Executable: `~/.local/bin/hfx` (or `$XDG_BIN_HOME/hfx`, or `--bin-dir /absolute/path`).
- Application-menu launcher: `$XDG_DATA_HOME/applications/hfx.desktop`, normally `~/.local/share/applications/hfx.desktop`.
- Native window/dock icon and installed SVG/PNG icons in the user’s hicolor theme.
- Optional Desktop shortcut, using the configured/localized `XDG_DESKTOP_DIR` from `user-dirs.dirs` rather than assuming an English folder name.

Omit `--desktop-shortcut` if you only want the application-menu/dock entry. Find **hfx** in your application menu and pin it to the dock. GNOME may require a desktop-icons extension to show shortcuts on the wallpaper, and a shortcut may need **right-click → Allow Launching**. The installer attempts the trust metadata but this depends on desktop support.

Normal Linux launches also register an application-menu entry automatically if there is no existing user/system `hfx.desktop`; this works for portable/source builds without running the installer. That entry points to the executable you launched. Use the installer for a permanent location independent of this checkout. Existing launchers are not hijacked when running a different development build. Previews and headless CLI commands do not auto-register. Set `HFX_NO_DESKTOP_INTEGRATION=1` or pass `--no-desktop-integration` to opt out of automatic registration.

The Desktop launcher uses a small starter workspace at `$XDG_DATA_HOME/hfx/workspace` for a fresh profile—not your entire home directory. Existing saved projects/chats are preserved. Launching from a terminal still uses its current directory for a fresh profile.

Other installer options:

```bash
./scripts/install-linux.sh --no-build                 # use the existing release build
./scripts/install-linux.sh --binary /path/to/hfx      # install a prebuilt binary
./scripts/uninstall-linux.sh                         # preserve chats, settings and login credentials
```

Rerun the installer to update. Binary replacement is atomic, even if the old instance is running; restart hfx to load the update. Quit hfx before uninstalling. Uninstall does not delete saved chats, credentials, or workspace files. Install/uninstall refuse to replace a binary not managed by these scripts; desktop registration refuses to overwrite unrelated user launchers.

### Portable Linux bundle

```bash
./scripts/package-linux.sh
# Creates dist/hfx-<version>-linux-<architecture>.tar.gz and its SHA-256 checksum.
```

Extract the bundle and run `./install.sh --desktop-shortcut`; Rust is not required to install that prebuilt bundle. It includes `hfx`, installer/uninstaller scripts, and the license. This is a native build for the build host’s architecture/glibc, **not** a universal static binary, AppImage, or `.deb`/`.rpm`. It still requires compatible desktop/OpenGL runtime libraries. Bubblewrap is only needed for the optional strict command sandbox.

### Completion notifications and sound

On Linux, successful work produces a small in-app **Work complete** toast, a desktop notification, and a quiet two-note chime. This works while focused, unfocused, or minimized. Each chat’s automatically running queued follow-ups count as one batch: alerts fire after the last follow-up, not after every tool call or steering boundary. Paused follow-ups do not prevent an alert for the completed run. Stop and errors stay silent; previews never issue native alerts.

**Settings → Appearance → Completion alerts** has independent, persisted switches for desktop notifications and the completion sound, enabled by default. Notifications contain no prompt/output excerpts. Desktop **Do Not Disturb** controls notification banners. The chime is independent: mute it with the sound checkbox or system audio settings. Desktop-service availability still applies.

Native alerts use `notify-send` and an audio helper (`paplay`, `pw-play`, or `canberra-gtk-play`, in that order). On Debian/Ubuntu, `libnotify-bin` supplies `notify-send` and `pulseaudio-utils` supplies `paplay`; PipeWire installations may already have `pw-play`. The installer reports missing helpers but does not install system packages. The packaged chime is 0.48 seconds at low amplitude/reduced playback volume; the last-resort Canberra fallback uses the desktop’s completion sound. All helpers run on a detached worker with a timeout, never on the window event thread. Missing services/helpers do not interrupt a run. Native completion alerts are currently Linux-only; the in-app toast works on other platforms.

## Background responsiveness

Generation, steering acknowledgements, queued follow-ups, question timers, and background chat autosaves run in eframe’s logic hook, including when it skips painting a minimized/occluded window. Unfocused/hidden windows use a slower background tick and skip animated text reveal; updates catch up without losing their recorded order. Incoming event bursts have a per-tick count limit and a soft time budget so they yield back to the window event loop.

Linux OpenGL rendering disables blocking VSync waits. glutin documents that, on Wayland, waiting for a buffer swap in a hidden window can block until it is visible again—preventing the event loop from answering the desktop’s responsiveness watchdog. hfx uses repaint timers for its streaming/animation pacing instead. This is not a suppression of the desktop’s “Application Not Responding” warning; a driver override or a different UI stall can still cause that warning. Click **Wait** rather than **Terminate** if you want to allow an in-progress run to recover; force termination can lose work since the last save.

For clean-exit behavior and shutdown tracing, see [Shutdown and diagnostics](usage.md#shutdown-and-diagnostics).
