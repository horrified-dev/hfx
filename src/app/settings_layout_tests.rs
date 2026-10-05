//! Compact connection-card geometry and native model-picker regressions.
use super::*;

fn text_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing text: {label}"))
}

fn seed_models(app: &mut Harness) {
    let settings = &app.saved.settings;
    app.probe_config = format!(
        "{:?}|{}|{}",
        settings.provider,
        settings.base_url(),
        settings.key()
    );
    app.probe_result = Some(Ok((0..11)
        .map(|index| backend::ModelInfo {
            id: format!("local-model-{index}"),
            context_window: Some(32768),
        })
        .collect()));
}

fn connection(
    app: &mut Harness,
    ctx: &egui::Context,
    width: f32,
    height: f32,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(width, height),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| {
            prefs::style(ui);
            app.connection_settings(ui);
        },
    );
    output.textures_delta.clear();
    output
}

#[test]
fn connection_cards_have_no_large_gap_before_or_after_the_action() {
    for provider in [Provider::Llama, Provider::OpenAI, Provider::OpenRouter] {
        for (width, height, scale) in [
            (304.0, 540.0, 1.0),
            (372.0, 900.0, 1.0),
            (304.0, 1500.0, 2.0),
        ] {
            for state in ["idle", "connected", "error", "busy"] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(scale);
                let cc = eframe::CreationContext::_new_kittest(ctx.clone());
                let mut app = Harness::new(&cc, Some("settings".into()));
                app.saved.settings.provider = provider;
                seed_models(&mut app);
                let (hold, rx) = mpsc::channel();
                match state {
                    "idle" => app.probe_result = None,
                    "error" => {
                        app.probe_result =
                            Some(Err("Could not connect to the test endpoint.".into()))
                    }
                    "busy" => {
                        app.probe_result = None;
                        app.probe_rx = Some(rx);
                    }
                    _ => (),
                }
                let output = connection(&mut app, &ctx, width, height, 0.0, vec![]);
                let help = text_rect(
                    &output,
                    match provider {
                        Provider::Llama => "Use your llama-server alias, or discover models below.",
                        Provider::OpenAI => {
                            "Use a Responses-compatible reasoning model available to your API account."
                        }
                        _ => "Use a provider/model ID, or discover models below.",
                    },
                );
                let action = text_rect(
                    &output,
                    if state == "busy" {
                        "Connecting…"
                    } else {
                        "Test connection"
                    },
                );
                let gap = action.top() - help.bottom();
                assert!(
                    (8.0..=40.0).contains(&gap),
                    "{provider:?}, {state}, {width}x{height}: gap above action is {gap}"
                );
                if state == "connected" {
                    let status = text_rect(&output, "CONNECTED · 11 MODELS");
                    assert!(
                        status.top() - action.bottom() <= 40.0,
                        "gap below action: {status:?}, {action:?}"
                    );
                    let picker = text_rect(&output, "Choose a discovered model");
                    assert!(picker.top() - status.bottom() <= 32.0);
                    assert!(
                        picker.bottom() < 400.0,
                        "connection card should be compact independently of viewport height"
                    );
                } else if state == "error" {
                    assert!(
                        text_rect(&output, "Could not connect to the test endpoint.").top()
                            - action.bottom()
                            <= 44.0
                    );
                }
                drop(hold);
            }
        }
    }
}

#[test]
fn the_compact_llama_model_picker_still_selects_discovered_models() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("settings".into()));
    app.saved.settings.provider = Provider::Llama;
    seed_models(&mut app);
    let output = connection(&mut app, &ctx, 372.0, 600.0, 0.0, vec![]);
    let pos = text_rect(&output, "Choose a discovered model").center();
    connection(
        &mut app,
        &ctx,
        372.0,
        600.0,
        0.1,
        vec![
            egui::Event::PointerMoved(pos),
            pointer_button(pos, egui::PointerButton::Primary, true),
        ],
    );
    connection(
        &mut app,
        &ctx,
        372.0,
        600.0,
        0.2,
        vec![pointer_button(pos, egui::PointerButton::Primary, false)],
    );
    // A newly opened native popup measures its placement on the next frame.
    let output = connection(&mut app, &ctx, 372.0, 600.0, 0.25, vec![]);
    let pos = text_rect(&output, "local-model-1").center();
    connection(
        &mut app,
        &ctx,
        372.0,
        600.0,
        0.3,
        vec![
            egui::Event::PointerMoved(pos),
            pointer_button(pos, egui::PointerButton::Primary, true),
        ],
    );
    connection(
        &mut app,
        &ctx,
        372.0,
        600.0,
        0.4,
        vec![pointer_button(pos, egui::PointerButton::Primary, false)],
    );
    assert_eq!(app.saved.settings.model(), "local-model-1");
    assert_eq!(app.saved.settings.context_limit(), 32768);
}
