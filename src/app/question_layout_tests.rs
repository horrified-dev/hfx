//! The ask_user reply hint, entered text and caret share the field's centerline.
use super::*;

const HINT: &str = "Or write your own response";

fn reply_id(app: &Harness) -> Id {
    Id::new(("question_reply", &app.question.as_ref().unwrap().id))
}

fn assert_reply_alignment(
    output: &egui::FullOutput,
    ctx: &egui::Context,
    id: Id,
    label: &str,
    focused: bool,
) {
    let field = ctx
        .read_response(id)
        .expect("question reply field is visible")
        .rect;
    let text = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing reply text: {label}"));
    let tolerance = 1.0 / ctx.pixels_per_point();
    assert!(
        (text.center().y - field.center().y).abs() <= tolerance,
        "Reply text must be vertically centered: text={text:?}, field={field:?}, scale={}",
        ctx.pixels_per_point()
    );
    assert!(
        text.top() >= field.top() && text.bottom() <= field.bottom(),
        "Reply text must not be clipped vertically"
    );
    assert!(
        text.left() - field.left() >= 4.0,
        "Reply text needs an inset from the field edge"
    );
    if focused {
        let stroke = ctx.style_of(egui::Theme::Dark).visuals.text_cursor.stroke;
        let caret = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::LineSegment {
                    points,
                    stroke: line_stroke,
                } if *line_stroke == stroke
                    && (points[0].x - points[1].x).abs() < 0.01
                    && field.expand(1.0).contains(points[0])
                    && field.expand(1.0).contains(points[1]) =>
                {
                    Some(egui::Rect::from_two_pos(points[0], points[1]))
                }
                _ => None,
            })
            .expect("focused reply field has a visible caret");
        assert!(
            (caret.center().y - field.center().y).abs() <= tolerance,
            "Caret must align with the hint/entered text: caret={caret:?}, field={field:?}"
        );
    }
}

#[test]
fn question_reply_hint_text_and_caret_are_centered_at_small_large_and_scaled_sizes() {
    for (width, height) in [(1180.0, 820.0), (720.0, 540.0)] {
        for scale in [1.0, 1.5, 2.0] {
            for focused in [false, true] {
                for custom in ["", "A custom reply", "Café 世界 🌿"] {
                    let ctx = egui::Context::default();
                    ctx.set_pixels_per_point(scale);
                    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
                    let mut app = Harness::new(&cc, Some("question".into()));
                    app.saved.settings.reduced_motion = true;
                    app.question.as_mut().unwrap().custom = custom.into();
                    draw(
                        &mut app,
                        &ctx,
                        width,
                        height,
                        0.0,
                        vec![],
                        egui::Modifiers::NONE,
                    );
                    let id = reply_id(&app);
                    if focused {
                        ctx.memory_mut(|m| m.request_focus(id));
                    }
                    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
                    style.visuals.text_cursor.blink = false;
                    ctx.set_style_of(egui::Theme::Dark, style);
                    let output = draw(
                        &mut app,
                        &ctx,
                        width,
                        height,
                        0.2,
                        vec![],
                        egui::Modifiers::NONE,
                    );
                    assert_reply_alignment(
                        &output,
                        &ctx,
                        id,
                        if custom.is_empty() { HINT } else { custom },
                        focused,
                    );
                }
            }
        }
    }
}
