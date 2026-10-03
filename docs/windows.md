# Windows installation

[Back to README](../README.md)

## Requirements

- Windows with a desktop session and a graphics driver supporting the app's OpenGL renderer.
- **Windows PowerShell 5.1** or **PowerShell 7**.
- For source builds: **Rust 1.95+** with the **MSVC toolchain** and Visual Studio Build Tools' **Desktop development with C++** workload (including a Windows SDK). Reopen the terminal after installing tools so `cargo` is on PATH.
- Run the installer from a complete checkout. It does not download an executable or install system packages.

## Install and update

From PowerShell in the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-windows.ps1 -DesktopShortcut
```

`-ExecutionPolicy Bypass` applies **only to that PowerShell process**, not the user's or machine's persistent policy. Review the checked-out scripts before running them. Group Policy or endpoint security can still restrict execution; the installer does not change those controls. PowerShell 7 users may substitute `pwsh` for `powershell`.

The script builds a release binary and installs **for the current user**, with no administrator prompt:

- Files: `%LOCALAPPDATA%\Programs\hfx\hfx.exe`, `hfx.ico`, `LICENSE.txt`, and an installation marker.
- Start-menu entry: `hfx.lnk` in the user's Windows **Programs** known folder.
- Optional Desktop shortcut: `hfx.lnk` in the user's **DesktopDirectory** known folder, including a redirected/OneDrive Desktop. Omit `-DesktopShortcut` if you only want the Start-menu entry.

Launch **hfx** from the Start menu and pin it if desired. Shortcuts start in `%APPDATA%\hfx\workspace`, a small starter workspace for a fresh profile. Existing saved projects/chats are not reset. Terminal launches use their current directory. The installer leaves your PATH unchanged; optionally add the install directory to your **user** PATH for terminal launching.

Quit hfx before updating or uninstalling. Rerun the installer to update. Files are staged locally, and a failed update restores the previous files. A locked executable causes an error rather than killing the running app. Nonempty unmanaged installation directories, redirected installation files, and unrelated existing shortcuts are not overwritten. Earlier Desktop/custom shortcuts stay recorded when their flag is omitted on update.

### Existing builds and custom paths

```powershell
.\scripts\install-windows.ps1 -NoBuild
.\scripts\install-windows.ps1 -Binary 'C:\builds\hfx.exe'
.\scripts\install-windows.ps1 -InstallDir "$env:LOCALAPPDATA\Apps\hfx"
```

`-Binary` skips building; supply a compatible **Windows executable** for your architecture. `-NoBuild` uses `target\release\hfx.exe` (or `CARGO_TARGET_DIR`, with the `CARGO_BUILD_TARGET` subdirectory if set). For other custom Cargo target configurations, use `-Binary` explicitly. The script does not cross-compile, create an MSI, or add a trusted code signature.

`-ShortcutDir` and `-DesktopDir` accept absolute filesystem directories when custom shortcut locations are needed. Their defaults use Windows known folders, not hard-coded English folder names. Custom install/shortcut paths can contain spaces and Unicode.

### Windows security prompts

Locally built or downloaded executables are **not Authenticode signed** by this project. SmartScreen or endpoint protection may warn or block them. Follow your organization's policy and only run builds you trust; the installer does not disable SmartScreen, antivirus, or file-origin checks.

## Uninstall without deleting data

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\uninstall-windows.ps1
# For a custom installation directory:
.\scripts\uninstall-windows.ps1 -InstallDir "$env:LOCALAPPDATA\Apps\hfx"
```

The uninstaller removes only the script-managed executable, icon, license, marker, and shortcuts that still target this installation and carry its ownership marker. It never recursively removes the installation directory: extra user files remain. A shortcut replaced by the user is preserved.

Chats, preferences, and the Codex OAuth cache in `%APPDATA%\hfx\data`, and files in `%APPDATA%\hfx\workspace`, are **not deleted**. Keep the checkout's installer/uninstaller scripts available for updates and removal.

## Platform limitations

Native completion notifications/sound and strict command confinement are currently Linux-only. The in-app completion toast works on Windows; trusted commands use `cmd.exe` and inherit the environment available to hfx. See [Tools & security](tools.md) before granting command access.
