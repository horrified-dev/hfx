//! Native geometry and interaction regressions for list/table blocks.
use super::*;
use egui::{Event, Modifiers, PointerButton, pos2, vec2};

const REPORT: &str = "Implemented **task #1: message-level viewport virtualization**.\n\n- Stable off-screen messages now reserve their exact measured height instead of rebuilding widgets every frame.\n- Preserves bottom-follow, disclosures, selection, keyboard focus, links, copying, and image viewing.\n- Remeasures changed content and invalidates measurements on resize, font/DPI changes, chat switches, and recovery.\n- Added **10 regression tests** and updated performance documentation.\n\n| Replies | Full layout | Virtualized |\n| --- | ---: | ---: |\n| 16 | 0.171 ms/frame | 0.067 ms/frame |\n| 64 | 0.637 ms/frame | 0.068 ms/frame |\n| 256 | 2.652 ms/frame | 0.077 ms/frame |\n| 1,024 | 10.850 ms/frame | 0.099 ms/frame |";

fn draw(
    ctx: &egui::Context,
    text: &str,
    width: f32,
    time: f64,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1600.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| show(ui, text, 14.0, TEXT),
    );
    output.textures_delta.clear();
    output
}

fn labels(output: &egui::FullOutput) -> Vec<&egui::epaint::TextShape> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn label<'a>(output: &'a egui::FullOutput, expected: &str) -> &'a egui::epaint::TextShape {
    labels(output)
        .into_iter()
        .find(|text| text.galley.text() == expected)
        .unwrap_or_else(|| {
            panic!(
                "missing label: {expected}; got {:?}",
                labels(output)
                    .iter()
                    .map(|t| t.galley.text())
                    .collect::<Vec<_>>()
            )
        })
}

fn bounds(text: &egui::epaint::TextShape) -> Rect {
    text.galley.rect.translate(text.pos.to_vec2())
}

fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn opened(output: &egui::FullOutput) -> Vec<&egui::OpenUrl> {
    output
        .platform_output
        .commands
        .iter()
        .filter_map(|cmd| match cmd {
            egui::OutputCommand::OpenUrl(url) => Some(url),
            _ => None,
        })
        .collect()
}

#[test]
fn the_report_renders_real_bullets_and_aligned_table_cells_at_wide_and_narrow_sizes() {
    for width in [720.0, 360.0, 280.0] {
        let ctx = egui::Context::default();
        theme::install(&ctx, 14.0, true);
        let output = draw(&ctx, REPORT, width, 0.0, vec![]);
        assert_eq!(
            labels(&output)
                .iter()
                .filter(|text| text.galley.text() == "•")
                .count(),
            4
        );
        let item = label(
            &output,
            "Stable off-screen messages now reserve their exact measured height instead of rebuilding widgets every frame.",
        );
        assert!(bounds(item).left() > 15.0);
        if width <= 360.0 {
            assert!(item.galley.rows.len() > 1);
        }
        for row in &item.galley.rows {
            assert_eq!(row.pos.x, 0.0, "wrapped rows use the same hanging indent");
        }
        for value in [
            "Replies",
            "Full layout",
            "Virtualized",
            "16",
            "64",
            "256",
            "1,024",
            "0.171 ms/frame",
            "0.077 ms/frame",
            "0.099 ms/frame",
        ] {
            label(&output, value);
        }
        let header = label(&output, "Replies");
        assert!(
            header
                .galley
                .job
                .sections
                .iter()
                .any(|section| section.format.extra_letter_spacing > 0.0)
        );
        assert!(
            bounds(label(&output, "Replies")).left() < bounds(label(&output, "Full layout")).left()
        );
        assert!(
            bounds(label(&output, "Full layout")).left()
                < bounds(label(&output, "Virtualized")).left()
        );
        assert!(
            (bounds(label(&output, "Full layout")).right()
                - bounds(label(&output, "0.171 ms/frame")).right())
            .abs()
                < 1.0
        );
        assert!(
            !labels(&output)
                .iter()
                .any(|text| text.galley.text().starts_with('|')
                    || text.galley.text().starts_with("- Stable"))
        );
    }
}

#[test]
fn nested_ordered_lists_and_continuation_paragraphs_keep_their_indentation() {
    let ctx = egui::Context::default();
    let output = draw(
        &ctx,
        "3. First\n1. Second\n  - Nested\n    continuation\n\n   Another parent paragraph\n4. Last\n\nAfter the list.",
        360.0,
        0.0,
        vec![],
    );
    label(&output, "3.");
    label(&output, "4.");
    label(&output, "5.");
    let parent = label(&output, "First");
    let child = label(&output, "Nested continuation");
    assert!(bounds(child).left() > bounds(parent).left());
    assert_eq!(
        bounds(label(&output, "Another parent paragraph")).left(),
        bounds(parent).left()
    );
    assert_eq!(bounds(label(&output, "After the list.")).left(), 0.0);
}

#[test]
fn tables_preserve_alignment_escaped_pipes_missing_cells_and_extra_data() {
    let ctx = egui::Context::default();
    let output = draw(
        &ctx,
        "Left | Center | Right\n:--- | :---: | ---:\none | café | 42\n`a\\|b` | x\\|y\nextra | cell | value | never drop this\n\nDone",
        600.0,
        0.0,
        vec![],
    );
    label(&output, "a|b");
    label(&output, "x|y");
    assert!(
        (bounds(label(&output, "Center")).center().x - bounds(label(&output, "café")).center().x)
            .abs()
            < 1.0
    );
    assert!(
        (bounds(label(&output, "Right")).right() - bounds(label(&output, "42")).right()).abs()
            < 1.0
    );
    label(&output, "extra | cell | value | never drop this");
    label(&output, "Done");
}

#[test]
fn malformed_partial_tables_and_fenced_blocks_stay_literal() {
    let ctx = egui::Context::default();
    let source = "| A | B |\n| --- | unfinished |";
    let output = draw(&ctx, source, 600.0, 0.0, vec![]);
    label(&output, "| A | B |");
    label(&output, "| --- | unfinished |");
    let literal =
        "- literal bullet\n| A | B |\n| --- | --- |\n| [literal](https://example.com) | `code` |";
    let output = draw(
        &ctx,
        &format!("```markdown\n{literal}\n```"),
        600.0,
        1.0,
        vec![],
    );
    label(&output, literal);
    assert!(!labels(&output).iter().any(|text| text.galley.text() == "•"));
    assert!(opened(&output).is_empty());
}

#[test]
fn table_and_list_links_relayout_after_resize_zoom_and_font_changes() {
    for source in [
        "- Read [日本語 café long linked label](https://example.com/docs)",
        "| Documentation | Other |\n| --- | --- |\n| [日本語 café long linked label](https://example.com/docs) | value |",
    ] {
        let ctx = egui::Context::default();
        theme::install(&ctx, 14.0, true);
        for (index, (width, scale)) in [(600.0, 1.0), (240.0, 1.5), (400.0, 2.0)]
            .into_iter()
            .enumerate()
        {
            ctx.set_pixels_per_point(scale);
            let time = index as f64 * 2.0;
            draw(&ctx, source, width, time, vec![]);
            let output = draw(&ctx, source, width, time + 0.1, vec![]);
            let expected = if source.starts_with('-') {
                "Read 日本語 café long linked label"
            } else {
                "日本語 café long linked label"
            };
            let text = label(&output, expected);
            let parsed = inline_text(
                if source.starts_with('-') {
                    &source[2..]
                } else {
                    "[日本語 café long linked label](https://example.com/docs)"
                },
                14.0,
                TEXT,
                false,
            );
            let rect = link_rects(&text.galley, text.pos, &parsed.links[0].characters)[0];
            draw(
                &ctx,
                source,
                width,
                time + 0.2,
                pointer(rect.center(), true),
            );
            let clicked = draw(
                &ctx,
                source,
                width,
                time + 0.3,
                pointer(rect.center(), false),
            );
            assert_eq!(opened(&clicked).len(), 1);
            assert_eq!(opened(&clicked)[0].url, "https://example.com/docs");
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.families.insert(
            egui::FontFamily::Proportional,
            fonts.families[&egui::FontFamily::Monospace].clone(),
        );
        ctx.set_fonts(fonts);
        draw(&ctx, source, 400.0, 8.0, vec![]);
        let output = draw(&ctx, source, 400.0, 9.0, vec![]);
        assert!(!labels(&output).is_empty());
    }
}

#[test]
fn table_and_list_link_text_can_be_selected_and_copied_without_opening_the_url() {
    for source in [
        "- [select this link](https://example.com/docs)",
        "| A | B |\n| --- | --- |\n| [select this link](https://example.com/docs) | value |",
    ] {
        let ctx = egui::Context::default();
        let output = draw(&ctx, source, 600.0, 0.0, vec![]);
        let text = label(&output, "select this link");
        let rect = bounds(text);
        let start = pos2(rect.left() + 1.0, rect.center().y);
        let end = pos2(rect.right() - 1.0, rect.center().y);
        draw(&ctx, source, 600.0, 0.1, pointer(start, true));
        draw(&ctx, source, 600.0, 0.2, vec![Event::PointerMoved(end)]);
        let released = draw(&ctx, source, 600.0, 0.3, pointer(end, false));
        assert!(opened(&released).is_empty());
        let copied = draw(&ctx, source, 600.0, 0.4, vec![Event::Copy]);
        assert!(copied.platform_output.commands.iter().any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if text.contains("select this link"))));
    }
}

#[test]
fn wide_tables_scroll_horizontally_and_recompute_link_hitboxes() {
    let ctx = egui::Context::default();
    let source = "| A | B | C | Last |\n| --- | --- | --- | --- |\n| one | two | three | [open last](https://example.com/last) |";
    draw(&ctx, source, 200.0, 0.0, vec![]);
    let initial = draw(&ctx, source, 200.0, 0.1, vec![]);
    assert!(
        !labels(&initial)
            .iter()
            .any(|text| text.galley.text() == "open last")
    );
    draw(
        &ctx,
        source,
        200.0,
        0.2,
        vec![
            Event::PointerMoved(pos2(100.0, 25.0)),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(-200.0, 0.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    for index in 0..10 {
        draw(&ctx, source, 200.0, 0.3 + index as f64 * 0.1, vec![]);
    }
    let output = draw(&ctx, source, 200.0, 1.5, vec![]);
    let pos = bounds(label(&output, "open last")).center();
    assert!(pos.x < 200.0);
    draw(&ctx, source, 200.0, 1.6, pointer(pos, true));
    let clicked = draw(&ctx, source, 200.0, 1.7, pointer(pos, false));
    assert_eq!(opened(&clicked)[0].url, "https://example.com/last");
}

#[test]
fn list_and_table_links_keep_keyboard_access_and_unsafe_urls_remain_literal() {
    for source in [
        "- [docs](https://example.com/docs)",
        "| A | B |\n| --- | --- |\n| [docs](https://example.com/docs) | value |",
    ] {
        let ctx = egui::Context::default();
        draw(&ctx, source, 600.0, 0.0, vec![]);
        let key = |key| Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        draw(&ctx, source, 600.0, 0.1, vec![key(egui::Key::Tab)]);
        let activated = draw(&ctx, source, 600.0, 0.2, vec![key(egui::Key::Enter)]);
        assert_eq!(opened(&activated)[0].url, "https://example.com/docs");
    }
    let ctx = egui::Context::default();
    let output = draw(
        &ctx,
        "- [unsafe](javascript:alert(1))\n\n| A | B |\n| --- | --- |\n| [local](file:///etc/passwd) | value |",
        600.0,
        0.0,
        vec![],
    );
    label(&output, "[unsafe](javascript:alert(1))");
    label(&output, "[local](file:///etc/passwd)");
    assert!(opened(&output).is_empty());
}

#[test]
fn very_narrow_empty_and_large_numbered_items_do_not_overlap_or_panic() {
    let ctx = egui::Context::default();
    let output = draw(
        &ctx,
        "999999999. \n999999999. next\n\nDone",
        35.0,
        0.0,
        vec![],
    );
    let first = bounds(label(&output, "999999999."));
    let second = bounds(label(&output, "1000000000."));
    assert!(first.bottom() <= second.top());
    assert!(second.bottom() <= bounds(label(&output, "Done")).top());
}

#[test]
fn streamed_unicode_tables_never_panic_or_lose_final_cells() {
    let ctx = egui::Context::default();
    let source =
        "| Name | Description |\n| --- | ---: |\n| café 🌿 | [世界](https://example.com/docs) |";
    for (index, (end, _)) in source.char_indices().enumerate() {
        draw(&ctx, &source[..end], 280.0, index as f64 * 0.05, vec![]);
    }
    let output = draw(&ctx, source, 280.0, 10.0, vec![]);
    label(&output, "Name");
    label(&output, "Description");
    label(&output, "café 🌿");
    label(&output, "世界");
    assert!(
        !labels(&output)
            .iter()
            .any(|text| text.galley.text().starts_with('|'))
    );
}
