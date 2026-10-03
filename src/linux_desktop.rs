//! Per-user Linux desktop integration. No root access, shell interpolation or GUI needed.
use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const APP_ID: &str = "hfx";
const MARKER: &str = "X-Hfx-Managed=true";
const SVG: &[u8] = include_bytes!("../assets/hfx.svg");
const PNG: &[u8] = include_bytes!("../assets/hfx.png");
pub const CHIME: &[u8] = include_bytes!("../assets/work-complete.wav");

#[derive(Debug)]
pub struct Locations {
    pub home: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
}

impl Locations {
    pub fn from_env() -> io::Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| io::Error::other("HOME must be an absolute path"))?;
        let data = absolute_env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let config = absolute_env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        Ok(Self { home, data, config })
    }
    pub fn entry(&self) -> PathBuf {
        self.data
            .join("applications")
            .join(format!("{APP_ID}.desktop"))
    }
    pub fn svg(&self) -> PathBuf {
        self.data.join("icons/hicolor/scalable/apps/hfx.svg")
    }
    pub fn png(&self) -> PathBuf {
        self.data.join("icons/hicolor/256x256/apps/hfx.png")
    }
    pub fn workspace(&self) -> PathBuf {
        self.data.join("hfx/workspace")
    }
    pub fn sound(&self) -> PathBuf {
        self.data.join("hfx/work-complete.wav")
    }

    pub fn desktop(&self) -> io::Result<PathBuf> {
        if let Ok(contents) = fs::read_to_string(self.config.join("user-dirs.dirs")) {
            for line in contents.lines() {
                if let Some(value) = line.trim().strip_prefix("XDG_DESKTOP_DIR=") {
                    let path = user_directory(value, &self.home)?;
                    if path == self.home {
                        return Err(io::Error::other(
                            "Desktop folder is disabled; use the application-menu launcher instead",
                        ));
                    }
                    return Ok(path);
                }
            }
        }
        Ok(self.home.join("Desktop"))
    }
}

fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

// user-dirs.dirs is configuration data, NOT a shell script. Never source/eval it.
fn user_directory(value: &str, home: &Path) -> io::Result<PathBuf> {
    let value = value
        .trim()
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| io::Error::other("Invalid quoted XDG_DESKTOP_DIR"))?;
    let value = value.replace("\\\"", "\"").replace("\\\\", "\\");
    let path = if value == "$HOME" || value == "${HOME}" {
        home.to_owned()
    } else if let Some(relative) = value
        .strip_prefix("$HOME/")
        .or_else(|| value.strip_prefix("${HOME}/"))
    {
        home.join(relative)
    } else {
        PathBuf::from(&value)
    };
    if !path.is_absolute() || value.contains(['`', '\n', '\r']) || value.contains("$(") {
        return Err(io::Error::other(
            "Desktop folder must be a literal absolute path or $HOME-relative path",
        ));
    }
    if path == home.join("") || path == home.join(".") {
        return Ok(home.to_owned());
    }
    Ok(path)
}

fn path_text(path: &Path) -> io::Result<&str> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::other("Desktop launcher paths must be UTF-8"))?;
    if !path.is_absolute() || text.chars().any(char::is_control) {
        return Err(io::Error::other(
            "Desktop launcher paths must be absolute and contain no control characters",
        ));
    }
    Ok(text)
}

fn value_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
}

fn exec_escape(path: &Path) -> io::Result<String> {
    let mut quoted = String::from("\"");
    for character in path_text(path)?.chars() {
        match character {
            '%' => quoted.push_str("%%"), // Never interpret filenames as desktop field codes.
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(character);
            }
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    // Desktop-value escaping happens before Exec's own quote/escape parsing.
    Ok(value_escape(&quoted))
}

fn entry_contents(executable: &Path, locations: &Locations) -> io::Result<String> {
    Ok(format!(
        "[Desktop Entry]\nVersion=1.0\nType=Application\nName=hfx\nGenericName=AI Coding Workspace\nComment=Your native AI coding workspace\nExec={}\nTryExec={}\nIcon={}\nPath={}\nTerminal=false\nStartupNotify=false\nStartupWMClass=hfx\nCategories=Development;\nKeywords=AI;coding;Codex;workspace;\n{MARKER}\n",
        exec_escape(executable)?,
        value_escape(path_text(executable)?),
        value_escape(path_text(&locations.svg())?),
        value_escape(path_text(&locations.workspace())?)
    ))
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn managed(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text.lines().any(|line| line == MARKER))
}

fn replace(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(io::Error::other(
            "Refusing to replace a desktop-integration symlink",
        ));
    }
    if fs::read(path).is_ok_and(|old| old == bytes) {
        // Do not churn desktop cache mtimes on repeat launches/installs.
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid integration path"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".hfx-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn register(
    locations: &Locations,
    executable: &Path,
    desktop_shortcut: bool,
) -> io::Result<()> {
    let entry = locations.entry();
    if exists(&entry) && !managed(&entry) {
        return Err(io::Error::other(
            "Refusing to replace an unmanaged hfx.desktop launcher",
        ));
    }
    let contents = entry_contents(executable, locations)?;
    // Check both destinations before changing either launcher.
    let candidate = locations.desktop().ok().map(|p| p.join("hfx.desktop"));
    let desktop = if desktop_shortcut {
        Some(locations.desktop()?.join("hfx.desktop"))
    } else {
        candidate.filter(|p| managed(p)) // Update an earlier shortcut without creating a new one.
    };
    if desktop.as_ref().is_some_and(|p| exists(p) && !managed(p)) {
        return Err(io::Error::other(
            "Refusing to replace an unmanaged Desktop/hfx.desktop",
        ));
    }
    fs::create_dir_all(locations.workspace())?;
    replace(&locations.svg(), SVG, 0o644)?;
    replace(&locations.png(), PNG, 0o644)?;
    replace(&locations.sound(), CHIME, 0o644)?;
    replace(&entry, contents.as_bytes(), 0o644)?;
    if let Some(desktop) = desktop {
        replace(&desktop, contents.as_bytes(), 0o755)?;
        // Trust metadata is optional (GNOME may require "Allow Launching").
        crate::notifications::run_helper(
            "gio",
            &[
                desktop.as_os_str().to_owned(),
                "metadata::trusted".into(),
                "true".into(),
            ],
            Some("set"),
        );
        println!("Desktop shortcut: {}", desktop.display());
    }
    println!("Application launcher: {}", entry.display());
    Ok(())
}

pub fn unregister(locations: &Locations) -> io::Result<()> {
    for path in [
        Some(locations.entry()),
        locations.desktop().ok().map(|p| p.join("hfx.desktop")),
    ]
    .into_iter()
    .flatten()
    {
        if managed(&path) {
            fs::remove_file(path)?;
        }
    }
    // Never remove chats, credentials, settings, or the starter workspace.
    for (path, expected) in [
        (locations.svg(), SVG),
        (locations.png(), PNG),
        (locations.sound(), CHIME),
    ] {
        if fs::read(&path).is_ok_and(|bytes| bytes == expected) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn start_auto_registration() {
    if std::env::var_os("HFX_NO_DESKTOP_INTEGRATION").is_some_and(|v| v == "1") {
        return;
    }
    // Registration/filesystem helpers must never delay the GUI event thread.
    std::thread::spawn(|| {
        let result = (|| {
            let locations = Locations::from_env()?;
            if exists(&locations.entry()) {
                return Ok(());
            }
            let system_dirs = std::env::var_os("XDG_DATA_DIRS")
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            if std::env::split_paths(&system_dirs)
                .filter(|p| p.is_absolute())
                .any(|p| p.join("applications/hfx.desktop").exists())
            {
                return Ok(());
            }
            register(&locations, &std::env::current_exe()?, false)
        })();
        if let Err(error) = result {
            eprintln!("Could not register hfx's Linux launcher: {error}");
        }
    });
}

pub fn handle_cli(args: &[OsString]) -> Option<io::Result<()>> {
    let command = args.first()?.to_str()?;
    if !matches!(command, "--install-desktop" | "--uninstall-desktop") {
        return None;
    }
    Some((|| {
        let locations = Locations::from_env()?;
        if command == "--uninstall-desktop" {
            if args.len() != 1 {
                return Err(io::Error::other(
                    "--uninstall-desktop takes no additional arguments",
                ));
            }
            unregister(&locations)
        } else {
            if args[1..].iter().any(|arg| arg != "--desktop-shortcut") {
                return Err(io::Error::other(
                    "Usage: hfx --install-desktop [--desktop-shortcut]",
                ));
            }
            register(&locations, &std::env::current_exe()?, args.len() > 1)
        }
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn locations(root: &Path) -> Locations {
        Locations {
            home: root.join("home"),
            data: root.join("data space"),
            config: root.join("config"),
        }
    }
    #[test]
    fn registration_is_valid_idempotent_and_uninstall_preserves_chats() {
        let root = tempfile::tempdir().unwrap();
        let locations = locations(root.path());
        let exe = root.path().join("bin space/hfx");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"binary").unwrap();
        register(&locations, &exe, false).unwrap();
        let before = fs::read(locations.entry()).unwrap();
        register(&locations, &exe, false).unwrap();
        assert_eq!(fs::read(locations.entry()).unwrap(), before);
        if let Ok(result) = std::process::Command::new("desktop-file-validate")
            .arg(locations.entry())
            .output()
        {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let chats = locations.data.join("hfx/chats.json");
        fs::write(&chats, "keep me").unwrap();
        unregister(&locations).unwrap();
        assert!(!locations.entry().exists());
        assert_eq!(fs::read_to_string(chats).unwrap(), "keep me");
    }
    #[test]
    fn localized_desktop_paths_are_parsed_without_evaluating_shell() {
        let root = tempfile::tempdir().unwrap();
        let locations = locations(root.path());
        fs::create_dir_all(&locations.config).unwrap();
        fs::write(
            locations.config.join("user-dirs.dirs"),
            "XDG_DESKTOP_DIR=\"$HOME/Bureau personnel\"\n",
        )
        .unwrap();
        assert_eq!(
            locations.desktop().unwrap(),
            locations.home.join("Bureau personnel")
        );
        assert!(user_directory("\"$(touch bad)\"", &locations.home).is_err());
        fs::write(
            locations.config.join("user-dirs.dirs"),
            "XDG_DESKTOP_DIR=\"$HOME/\"\n",
        )
        .unwrap();
        assert!(locations.desktop().is_err());
    }
    #[test]
    fn exec_paths_escape_percent_quotes_and_shell_metacharacters() {
        assert_eq!(
            exec_escape(Path::new("/tmp/a %f/hfx")).unwrap(),
            "\"/tmp/a %%f/hfx\""
        );
        assert_eq!(
            exec_escape(Path::new("/tmp/\"$`\\/hfx")).unwrap(),
            "\"/tmp/\\\\\"\\\\$\\\\`\\\\\\\\/hfx\""
        );
        assert!(exec_escape(Path::new("/tmp/bad\n/hfx")).is_err());
    }
    #[test]
    fn existing_unmanaged_launchers_are_not_overwritten_or_removed() {
        let root = tempfile::tempdir().unwrap();
        let locations = locations(root.path());
        fs::create_dir_all(locations.entry().parent().unwrap()).unwrap();
        fs::write(locations.entry(), "foreign launcher").unwrap();
        assert!(register(&locations, &root.path().join("hfx"), false).is_err());
        unregister(&locations).unwrap();
        assert_eq!(
            fs::read_to_string(locations.entry()).unwrap(),
            "foreign launcher"
        );
    }
}
