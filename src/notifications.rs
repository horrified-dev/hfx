//! Completion alerts use desktop helpers off the GUI and inference threads.
#[cfg(target_os = "linux")]
use std::{
    ffi::OsString,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
pub fn run_helper(program: &str, args: &[OsString], first: Option<&str>) -> bool {
    let mut command = Command::new(program);
    if let Some(first) = first {
        command.arg(first);
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for key in ["OPENAI_API_KEY", "OPENROUTER_API_KEY", "LLAMA_API_KEY"] {
        command.env_remove(key);
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn deliver(
    notify: bool,
    sound: bool,
    path: Option<OsString>,
    mut run: impl FnMut(&str, &[OsString]) -> bool,
) {
    if notify {
        // No prompt/output excerpts: notifications can appear on a lock screen.
        // suppress-sound avoids a second chime from the notification daemon.
        run(
            "notify-send",
            &[
                "--app-name=hfx".into(),
                "--icon=hfx".into(),
                "--urgency=normal".into(),
                "--expire-time=6000".into(),
                "--hint=string:desktop-entry:hfx".into(),
                "--hint=boolean:suppress-sound:true".into(),
                "--".into(),
                "hfx — Work complete".into(),
                "Your task is finished. Open hfx to review the result.".into(),
            ],
        );
    }
    if sound {
        if let Some(path) = path {
            if run("paplay", &["--volume=49152".into(), path.clone()]) {
                return;
            }
            if run("pw-play", &["--volume=0.5".into(), path]) {
                return;
            }
        }
        run(
            "canberra-gtk-play",
            &[
                "--id=complete".into(),
                "--description=hfx work complete".into(),
            ],
        );
    }
}

#[cfg(target_os = "linux")]
fn prepare_sound() -> Option<OsString> {
    crate::linux_desktop::Locations::from_env()
        .ok()
        .and_then(|locations| {
            let path = locations.sound();
            std::fs::create_dir_all(path.parent()?).ok()?;
            if !path.exists() {
                std::fs::write(&path, crate::linux_desktop::CHIME).ok()?;
            }
            Some(path.into_os_string())
        })
}

pub fn completed(desktop_notification: bool, sound: bool) {
    #[cfg(target_os = "linux")]
    if desktop_notification || sound {
        // Detached and bounded: even a hung notification/audio server cannot
        // block painting, generation, or runtime shutdown.
        std::thread::spawn(move || {
            deliver(
                desktop_notification,
                sound,
                if sound { prepare_sound() } else { None },
                |program, args| run_helper(program, args, None),
            )
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (desktop_notification, sound);
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn notification_has_no_chat_content_and_sound_can_be_disabled() {
        let mut calls = Vec::new();
        deliver(true, false, None, |name, args| {
            calls.push((name.to_owned(), args.to_vec()));
            true
        });
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "notify-send");
        assert!(
            calls[0]
                .1
                .contains(&OsString::from("--hint=boolean:suppress-sound:true"))
        );
        assert!(
            calls[0]
                .1
                .contains(&OsString::from("--hint=string:desktop-entry:hfx"))
        );
    }
    #[test]
    fn disabled_alerts_do_not_launch_helpers() {
        deliver(false, false, None, |_, _| {
            panic!("disabled alerts must do nothing")
        });
    }
    #[test]
    fn sound_is_independent_and_stops_after_the_first_successful_player() {
        let mut calls = Vec::new();
        deliver(
            false,
            true,
            Some("/tmp/quiet chime.wav".into()),
            |name, _| {
                calls.push(name.to_owned());
                name == "pw-play"
            },
        );
        assert_eq!(calls, ["paplay", "pw-play"]);
    }
    #[test]
    fn a_hung_helper_is_killed_instead_of_blocking_indefinitely() {
        let started = Instant::now();
        assert!(!run_helper(
            "sh",
            &["-c".into(), "exec sleep 10".into()],
            None
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_helpers_fail_gracefully() {
        assert!(!run_helper(
            "hfx-nonexistent-notification-helper",
            &[],
            None
        ));
    }
}
