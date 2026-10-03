//! Lightweight, selectable chat Markdown with inline HTTP(S) links.
//! Blocks retain their layout while streaming; incomplete markup stays literal.
use std::ops::Range;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};

use crate::theme::{self, ACCENT, CODE, LINE, MUTED, RAIL, SURFACE, TEXT};

#[derive(Debug)]
struct InlineLink {
    characters: Range<usize>,
    label: String,
    url: String,
}

#[derive(Default)]
struct InlineText {
    job: egui::text::LayoutJob,
    links: Vec<InlineLink>,
    characters: usize,
}

#[derive(Clone, Copy)]
struct InlineStyle {
    size: f32,
    color: Color32,
    strong: bool,
    link: bool,
}

impl InlineText {
    fn append(&mut self, text: &str, style: InlineStyle, code: bool) {
        self.job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: if code {
                    FontId::monospace((style.size - 1.0).max(1.0))
                } else {
                    FontId::proportional(style.size)
                },
                color: if code || style.link {
                    ACCENT
                } else if style.strong {
                    TEXT
                } else {
                    style.color
                },
                background: if code { SURFACE } else { Color32::TRANSPARENT },
                extra_letter_spacing: if style.strong && !code { 0.2 } else { 0.0 },
                underline: if style.link {
                    Stroke::new(1.0, ACCENT)
                } else {
                    Stroke::NONE
                },
                ..Default::default()
            },
        );
        self.characters += text.chars().count();
    }
}

// Match balanced brackets/parentheses without treating escaped delimiters as syntax.
fn closing_delimiter(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut escaped = false;
    for (offset, ch) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(offset);
            }
        }
    }
    None
}

// Code spans are literal, including Markdown links and backslash escapes.
fn code_span(text: &str) -> Option<(&str, usize)> {
    let ticks = text.bytes().take_while(|&ch| ch == b'`').count();
    let mut offset = ticks;
    while offset < text.len() {
        let start = offset + text[offset..].find('`')?;
        let count = text[start..].bytes().take_while(|&ch| ch == b'`').count();
        if count == ticks {
            return Some((&text[ticks..start], start + ticks));
        }
        offset = start + count;
    }
    None
}

fn unescape(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            result.push(chars.next().unwrap());
        } else {
            result.push(ch);
        }
    }
    result
}

fn link_parts(text: &str) -> Option<(&str, &str, usize)> {
    let label_end = closing_delimiter(text, '[', ']')?;
    let destination = text[label_end + 1..].strip_prefix('(')?;
    let end = closing_delimiter(&text[label_end + 1..], '(', ')')?;
    let inner = destination[..end - 1].trim();
    let (url, title) = if let Some(angled) = inner.strip_prefix('<') {
        let close = angled.find('>')?;
        (&angled[..close], angled[close + 1..].trim())
    } else if let Some(space) = inner.find(char::is_whitespace) {
        (&inner[..space], inner[space..].trim())
    } else {
        (inner, "")
    };
    if !title.is_empty()
        && ![("\"", "\""), ("'", "'"), ("(", ")")]
            .iter()
            .any(|(a, b)| title.len() >= 2 && title.starts_with(a) && title.ends_with(b))
    {
        return None;
    }
    Some((&text[1..label_end], url, label_end + end + 2))
}

fn safe_url(destination: &str) -> Option<String> {
    let url = unescape(destination);
    if url.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return None;
    }
    let parsed = reqwest::Url::parse(&url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(url)
}

fn inline_spans(out: &mut InlineText, text: &str, style: InlineStyle, depth: usize) {
    if depth >= 8 {
        out.append(text, style, false);
        return;
    }
    let mut offset = 0;
    let mut plain = 0;
    while offset < text.len() {
        let rest = &text[offset..];
        let ch = rest.chars().next().unwrap();
        let mut consumed = 0;
        if ch == '\\' {
            if let Some(escaped) = rest.chars().nth(1).filter(char::is_ascii_punctuation) {
                out.append(&text[plain..offset], style, false);
                out.append(&rest[1..1 + escaped.len_utf8()], style, false);
                consumed = 1 + escaped.len_utf8();
            }
        } else if ch == '`' {
            if let Some((body, length)) = code_span(rest) {
                out.append(&text[plain..offset], style, false);
                out.append(body, style, true);
                consumed = length;
            }
        } else if let Some(after) = rest.strip_prefix("**") {
            if let Some(end) = after.find("**") {
                out.append(&text[plain..offset], style, false);
                inline_spans(
                    out,
                    &after[..end],
                    InlineStyle {
                        strong: true,
                        ..style
                    },
                    depth + 1,
                );
                consumed = end + 4;
            }
        } else if rest.starts_with("![") {
            // Inline images are not loaded or opened as links. Keep their syntax visible.
            if let Some((_, _, length)) = link_parts(&rest[1..]) {
                out.append(&text[plain..offset], style, false);
                out.append(&rest[..length + 1], style, false);
                consumed = length + 1;
            }
        } else if ch == '['
            && !style.link
            && let Some((label, destination, length)) = link_parts(rest)
            && let Some(url) = safe_url(destination)
        {
            out.append(&text[plain..offset], style, false);
            let characters = out.characters;
            let bytes = out.job.text.len();
            inline_spans(
                out,
                label,
                InlineStyle {
                    link: true,
                    ..style
                },
                depth + 1,
            );
            if out.characters > characters {
                out.links.push(InlineLink {
                    characters: characters..out.characters,
                    label: out.job.text[bytes..].to_owned(),
                    url,
                });
            }
            consumed = length;
        }

        if consumed > 0 {
            offset += consumed;
            plain = offset;
        } else {
            offset += ch.len_utf8();
        }
    }
    out.append(&text[plain..], style, false);
}

fn inline_text(text: &str, size: f32, color: Color32, strong: bool) -> InlineText {
    let mut out = InlineText::default();
    inline_spans(
        &mut out,
        text,
        InlineStyle {
            size,
            color,
            strong,
            link: false,
        },
        0,
    );
    out
}

// Use the actual wrapped glyphs, not a bounding box that also covers nearby prose.
fn link_rects(galley: &egui::Galley, pos: Pos2, characters: &Range<usize>) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut row_start = 0;
    for row in &galley.rows {
        let row_end = row_start + row.glyphs.len();
        let start = characters.start.max(row_start);
        let end = characters.end.min(row_end);
        if start < end {
            let first = &row.glyphs[start - row_start];
            let last = &row.glyphs[end - row_start - 1];
            rects.push(Rect::from_min_max(
                pos + row.pos.to_vec2() + egui::vec2(first.pos.x, 0.0),
                pos + row.pos.to_vec2() + egui::vec2(last.max_x(), row.size.y),
            ));
        }
        row_start = row_end + usize::from(row.ends_with_newline);
    }
    rects
}

fn inline_label(ui: &mut Ui, text: &str, size: f32, color: Color32, strong: bool) {
    let inline = inline_text(text, size, color, strong);
    let (pos, galley, response) = egui::Label::new(inline.job)
        .wrap()
        .selectable(true)
        .layout_in_ui(ui);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text())
    });
    if !ui.is_rect_visible(response.rect) {
        return;
    }
    egui::text_selection::LabelSelectionState::label_text_selection(
        ui,
        &response,
        pos,
        galley.clone(),
        color,
        Stroke::NONE,
    );
    for (link_index, link) in inline.links.iter().enumerate() {
        for (row_index, rect) in link_rects(&galley, pos, &link.characters)
            .into_iter()
            .enumerate()
        {
            let hit = ui.interact(
                rect,
                response.id.with(("markdown-link", link_index, row_index)),
                Sense::click(),
            );
            hit.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), &link.label)
            });
            if hit.hovered() || hit.has_focus() {
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    Stroke::new(1.0, ACCENT),
                );
            }
            if hit.clicked_with_open_in_background() {
                ui.open_url(egui::OpenUrl {
                    url: link.url.clone(),
                    new_tab: true,
                });
            } else if hit.clicked() {
                ui.open_url(egui::OpenUrl {
                    url: link.url.clone(),
                    new_tab: false,
                });
            }
            hit.on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(&link.url);
        }
    }
}

/// Stable block layout while streaming. Code blocks can be copied independently.
pub fn show(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    let mut code: Option<(String, String)> = None;
    for line in text.split('\n') {
        if let Some(language) = line.strip_prefix("```") {
            if let Some((language, body)) = code.take() {
                code_block(ui, &language, &body);
            } else {
                code = Some((language.to_owned(), String::new()));
            }
        } else if let Some((_, body)) = &mut code {
            body.push_str(line);
            body.push('\n');
        } else if line.is_empty() {
            ui.add_space(5.0);
        } else if let Some(heading) = line
            .strip_prefix("### ")
            .or_else(|| line.strip_prefix("## "))
            .or_else(|| line.strip_prefix("# "))
        {
            ui.add_space(8.0);
            inline_label(ui, heading, size + 2.0, TEXT, true);
        } else {
            inline_label(ui, line, size, color, false);
        }
    }
    if let Some((language, body)) = code {
        code_block(ui, &language, &body);
    }
}

fn code_block(ui: &mut Ui, language: &str, body: &str) {
    egui::Frame::NONE
        .fill(RAIL)
        .corner_radius(9)
        .stroke(Stroke::new(1.0, LINE))
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 1.0).max(0.0));
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(if language.is_empty() {
                        "code"
                    } else {
                        language
                    })
                    .size(11.0)
                    .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, theme::Icon::Copy, "Copy code", false, 24.0).clicked()
                    {
                        ui.ctx().copy_text(body.trim_end().into());
                    }
                });
            });
            ui.add_space(5.0);
            egui::ScrollArea::horizontal()
                .id_salt(ui.id().with(body.len()))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(body.trim_end()).monospace().color(CODE),
                        )
                        .selectable(true),
                    );
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, FontFamily, Modifiers, PointerButton, pos2, vec2};

    const COMMIT: &str =
        "https://github.com/horrified-dev/hfx/commit/9e9b32d61e9cf50c9b5f598110fd40a8693111cb";

    #[test]
    fn markdown_commit_link_renders_only_its_formatted_label() {
        let text = format!("Commit: [`9e9b32d`]({COMMIT}) — `docs: streamline README`");
        let parsed = inline_text(&text, 14.0, TEXT, false);
        assert_eq!(parsed.job.text, "Commit: 9e9b32d — docs: streamline README");
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].characters, 8..15);
        assert_eq!(parsed.links[0].label, "9e9b32d");
        assert_eq!(parsed.links[0].url, COMMIT);
        let format = parsed.job.format_at_byte(egui::text::ByteIndex(8));
        assert_eq!(format.font_id.family, FontFamily::Monospace);
        assert_eq!(format.underline.color, ACCENT);
        assert_eq!(format.color, ACCENT);
    }

    #[test]
    fn markdown_links_support_unicode_nested_formatting_and_balanced_urls() {
        let parsed = inline_text(
            "日本語: **[café `λ`](https://example.com/a(b))** and [次へ](<https://example.com/next> \"Next\")",
            14.0,
            MUTED,
            false,
        );
        assert_eq!(parsed.job.text, "日本語: café λ and 次へ");
        assert_eq!(parsed.links.len(), 2);
        assert_eq!(parsed.links[0].characters, 5..11);
        assert_eq!(parsed.links[0].url, "https://example.com/a(b)");
        assert_eq!(parsed.links[1].label, "次へ");
        assert_eq!(parsed.links[1].url, "https://example.com/next");
        let escaped = inline_text(r"[docs](https://example.com/a\(b\))", 14.0, TEXT, false);
        assert_eq!(escaped.links[0].url, "https://example.com/a(b)");
    }

    #[test]
    fn markdown_code_escapes_and_images_are_not_activated_as_links() {
        for text in [
            "`[literal](https://example.com)`",
            "``[literal `code`](https://example.com)``",
            r"\[literal](https://example.com)",
            "![image](https://example.com/image.png)",
            "![image [label]](https://example.com/image.png)",
        ] {
            let parsed = inline_text(text, 14.0, TEXT, false);
            assert!(
                parsed.links.is_empty(),
                "literal markup was activated: {text}"
            );
        }
        assert_eq!(
            inline_text("`[literal](https://example.com)`", 14.0, TEXT, false)
                .job
                .text,
            "[literal](https://example.com)"
        );
        assert_eq!(
            inline_text(r"\[literal](https://example.com)", 14.0, TEXT, false)
                .job
                .text,
            "[literal](https://example.com)"
        );
    }

    #[test]
    fn markdown_unsafe_and_unsupported_destinations_stay_literal() {
        for destination in [
            "javascript:alert(1)",
            "data:text/html,bad",
            "file:///etc/passwd",
            "ssh://example.com",
            "https://user:password@example.com/",
            "https://",
            "README.md",
            "https://example.com/bad title",
        ] {
            let text = format!("[label]({destination})");
            let parsed = inline_text(&text, 14.0, TEXT, false);
            assert!(parsed.links.is_empty(), "unsafe link: {destination}");
            assert_eq!(parsed.job.text, text);
        }
        assert_eq!(
            inline_text("[local](http://127.0.0.1:8080/docs)", 14.0, TEXT, false)
                .links
                .len(),
            1
        );
    }

    #[test]
    fn markdown_streamed_links_remain_literal_until_complete() {
        let text = format!("Before [次へ]({COMMIT})");
        for (end, _) in text.char_indices() {
            let prefix = &text[..end];
            let parsed = inline_text(prefix, 14.0, TEXT, false);
            assert!(parsed.links.is_empty(), "partial link activated: {prefix}");
            assert_eq!(parsed.job.text, prefix);
        }
        let parsed = inline_text(&text, 14.0, TEXT, false);
        assert_eq!(parsed.job.text, "Before 次へ");
        assert_eq!(parsed.links.len(), 1);
    }

    fn draw(
        ctx: &egui::Context,
        text: &str,
        width: f32,
        time: f64,
        events: Vec<Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 400.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_max_width(width);
                show(ui, text, 14.0, TEXT);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn text_shape(output: &egui::FullOutput, expected: &str) -> egui::epaint::TextShape {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == expected => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing rendered text: {expected}"))
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
            .filter_map(|command| match command {
                egui::OutputCommand::OpenUrl(url) => Some(url),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn markdown_wrapped_unicode_links_open_only_the_clicked_destination() {
        let text = format!("Before [日本語 café long linked label]({COMMIT}) after");
        let parsed = inline_text(&text, 14.0, TEXT, false);
        for width in [90.0, 170.0, 440.0] {
            for scale in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                theme::install(&ctx, 14.0, true);
                ctx.set_pixels_per_point(scale);
                let initial = draw(&ctx, &text, width, 0.0, vec![]);
                assert!(opened(&initial).is_empty());
                let shape = text_shape(&initial, &parsed.job.text);
                let rects = link_rects(&shape.galley, shape.pos, &parsed.links[0].characters);
                assert!(!rects.is_empty());
                if width == 90.0 {
                    assert!(rects.len() > 1);
                }
                for (index, rect) in rects.iter().enumerate() {
                    let time = index as f64 + 0.1;
                    assert!(
                        opened(&draw(
                            &ctx,
                            &text,
                            width,
                            time,
                            pointer(rect.center(), true)
                        ))
                        .is_empty()
                    );
                    let clicked = draw(
                        &ctx,
                        &text,
                        width,
                        time + 0.1,
                        pointer(rect.center(), false),
                    );
                    let urls = opened(&clicked);
                    assert_eq!(
                        urls.len(),
                        1,
                        "missing click at width {width}, scale {scale}"
                    );
                    assert_eq!(urls[0].url, COMMIT);
                }
                let prose = shape.pos + vec2(3.0, shape.galley.rows[0].height() * 0.5);
                draw(&ctx, &text, width, 10.0, pointer(prose, true));
                assert!(opened(&draw(&ctx, &text, width, 10.1, pointer(prose, false))).is_empty());
            }
        }
    }

    #[test]
    fn markdown_link_drag_selects_text_without_opening_the_browser() {
        let ctx = egui::Context::default();
        theme::install(&ctx, 14.0, true);
        let text = "Before [select this link](https://example.com) after";
        let parsed = inline_text(text, 14.0, TEXT, false);
        let initial = draw(&ctx, text, 440.0, 0.0, vec![]);
        let shape = text_shape(&initial, &parsed.job.text);
        let rect = link_rects(&shape.galley, shape.pos, &parsed.links[0].characters)[0];
        let start = pos2(rect.left() + 1.0, rect.center().y);
        let end = pos2(rect.right() - 1.0, rect.center().y);
        assert!(opened(&draw(&ctx, text, 440.0, 0.1, pointer(start, true))).is_empty());
        assert!(
            opened(&draw(
                &ctx,
                text,
                440.0,
                0.2,
                vec![Event::PointerMoved(end)]
            ))
            .is_empty()
        );
        assert!(opened(&draw(&ctx, text, 440.0, 0.3, pointer(end, false))).is_empty());
        let copy = draw(&ctx, text, 440.0, 0.4, vec![Event::Copy]);
        let copied = copy
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .expect("dragged link text can be copied");
        assert!(copied.contains("select this link"));
        assert!(!copied.contains("https://"));
    }

    #[test]
    fn markdown_links_are_keyboard_accessible() {
        let ctx = egui::Context::default();
        theme::install(&ctx, 14.0, true);
        let text = "Read [documentation](https://example.com/docs)";
        draw(&ctx, text, 440.0, 0.0, vec![]);
        let key = |key| Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        draw(&ctx, text, 440.0, 0.1, vec![key(egui::Key::Tab)]);
        let output = draw(&ctx, text, 440.0, 0.2, vec![key(egui::Key::Enter)]);
        assert_eq!(opened(&output).len(), 1);
        assert_eq!(opened(&output)[0].url, "https://example.com/docs");
    }

    #[test]
    fn markdown_headings_parse_links_but_fenced_code_stays_literal() {
        let ctx = egui::Context::default();
        theme::install(&ctx, 14.0, true);
        let output = draw(
            &ctx,
            "## See [docs](https://example.com)\n\n```markdown\n[label](https://example.com)\n```",
            440.0,
            0.0,
            vec![],
        );
        text_shape(&output, "See docs");
        text_shape(&output, "[label](https://example.com)");
        assert!(opened(&output).is_empty());
    }
}
