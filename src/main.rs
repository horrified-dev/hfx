mod app;
mod attachments;
mod backend;
mod codex;
mod commands;
mod context;
mod lifecycle;
#[cfg(target_os = "linux")]
mod linux_desktop;
mod motion;
mod notifications;
mod persistence;
mod sandbox;
mod state;
mod theme;
mod tools;
mod web;

use eframe::egui;

fn window_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/hfx.png"))
        .expect("embedded hfx icon")
        .into_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

fn native_options(preview: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("hfx — your coding workspace")
            .with_app_id("hfx")
            .with_icon(window_icon())
            .with_decorations(false)
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([720.0, 540.0]),
        renderer: eframe::Renderer::Glow,
        // Wayland may stop sending frame callbacks for a covered/hidden window.
        // glutin documents that SwapInterval::Wait can then block swap_buffers
        // (and the UI event loop) until the window is visible again. Let hfx's
        // repaint timers pace Linux updates instead; do not disable WM ping checks.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: !cfg!(target_os = "linux"),
            ..Default::default()
        },
        // Preview runs never replace a user's saved sessions.
        persistence_path: preview.then(|| std::env::temp_dir().join("hfx-preview.ron")),
        ..Default::default()
    }
}

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        println!("hfx {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "hfx — your coding workspace\n\nLinux: --install-desktop [--desktop-shortcut], --uninstall-desktop\nOther options: --version, --no-desktop-integration, --preview=NAME"
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if let Some(result) = linux_desktop::handle_cli(&args) {
        if let Err(error) = result {
            eprintln!("Desktop integration failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let preview = args
        .iter()
        .filter_map(|a| a.to_str())
        .find_map(|a| a.strip_prefix("--preview=").map(str::to_owned));
    #[cfg(target_os = "linux")]
    if preview.is_none() && !args.iter().any(|a| a == "--no-desktop-integration") {
        linux_desktop::start_auto_registration();
    }
    let options = native_options(preview.is_some());
    let result = eframe::run_native(
        "hfx",
        options,
        Box::new(move |cc| Ok(Box::new(app::Harness::new(cc, preview)))),
    );
    lifecycle::native_exit_finished();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_rendering_does_not_wait_for_linux_compositor_vsync() {
        let options = native_options(false);
        assert_eq!(options.glow_options.vsync, !cfg!(target_os = "linux"));
        assert!(matches!(options.renderer, eframe::Renderer::Glow));
        assert!(options.persistence_path.is_none());
    }

    #[test]
    fn native_window_has_the_same_app_id_as_the_desktop_launcher_and_an_icon() {
        let options = native_options(false);
        assert_eq!(options.viewport.app_id.as_deref(), Some("hfx"));
        let icon = options.viewport.icon.unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn preview_storage_stays_separate_with_the_same_rendering_policy() {
        let options = native_options(true);
        assert_eq!(
            options.glow_options.vsync,
            native_options(false).glow_options.vsync
        );
        assert_eq!(
            options.persistence_path,
            Some(std::env::temp_dir().join("hfx-preview.ron"))
        );
    }
}
