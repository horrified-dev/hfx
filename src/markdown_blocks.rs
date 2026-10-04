//! Small borrowed block parser for chat lists and pipe tables. Native labels keep
//! ownership of wrapping, selection, links, and font/DPI-dependent geometry.
use std::borrow::Cow;

use super::*;
use egui::{Align, Layout, UiBuilder, pos2, vec2};

const MAX_TABLE_COLUMNS: usize = 64;

type Lines<'a> = std::iter::Peekable<std::iter::Enumerate<std::str::Split<'a, char>>>;

#[derive(Debug)]
struct ListItem<'a> {
    indent: usize,
    content_indent: usize,
    number: Option<u64>,
    delimiter: char,
    body: &'a str,
}

struct ListLevel {
    indent: usize,
    content_indent: usize,
    text_offset: f32,
    next_number: Option<u64>,
    delimiter: char,
}

fn indentation(line: &str) -> (usize, &str) {
    indentation_from(line, 0)
}

fn indentation_from(line: &str, mut columns: usize) -> (usize, &str) {
    let text = line.trim_start_matches([' ', '\t']);
    for ch in line[..line.len() - text.len()].chars() {
        columns += if ch == '\t' { 4 - columns % 4 } else { 1 };
    }
    (columns, text)
}

fn list_item(line: &str) -> Option<ListItem<'_>> {
    let (indent, text) = indentation(line);
    let first = *text.as_bytes().first()?;
    // A thematic rule is not a list item. Keep unsupported rule syntax literal.
    if matches!(first, b'-' | b'*' | b'_')
        && text.bytes().filter(|ch| !ch.is_ascii_whitespace()).count() >= 3
        && text
            .bytes()
            .all(|ch| ch == first || ch.is_ascii_whitespace())
    {
        return None;
    }
    let (prefix, number, delimiter) = if matches!(first, b'-' | b'+' | b'*') {
        (1, None, char::from(first))
    } else {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        let delimiter = *text.as_bytes().get(digits)?;
        if !matches!(delimiter, b'.' | b')') {
            return None;
        }
        (
            digits + 1,
            Some(text[..digits].parse().ok()?),
            char::from(delimiter),
        )
    };
    let rest = &text[prefix..];
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let (content_indent, body) = indentation_from(rest, indent + prefix);
    Some(ListItem {
        indent,
        content_indent,
        number,
        delimiter,
        body,
    })
}

fn heading(line: &str) -> Option<&str> {
    line.strip_prefix("### ")
        .or_else(|| line.strip_prefix("## "))
        .or_else(|| line.strip_prefix("# "))
}

// Split only real delimiters, not escaped pipes or pipes inside code spans.
// The bound applies before allocating untrusted numbers of columns. Oversized
// or malformed rows fall back to literal text, never silently lose extra cells.
fn table_cells(line: &str) -> Option<Vec<&str>> {
    let line = line.trim();
    let mut cells = Vec::new();
    let mut offset = 0;
    let mut start = 0;
    let mut pipes = 0;
    while offset < line.len() {
        match line.as_bytes()[offset] {
            b'\\' => {
                offset += 1;
                if let Some(ch) = line[offset..].chars().next() {
                    offset += ch.len_utf8();
                }
            }
            b'`' => {
                offset += code_span(&line[offset..]).map_or(1, |(_, length)| length);
            }
            b'|' => {
                if offset != 0 {
                    cells.push(line[start..offset].trim());
                    if cells.len() > MAX_TABLE_COLUMNS {
                        return None;
                    }
                }
                pipes += 1;
                start = offset + 1;
                offset += 1;
            }
            _ => {
                offset += line[offset..].chars().next().unwrap().len_utf8();
            }
        }
    }
    if start < line.len() {
        cells.push(line[start..].trim());
    }
    (pipes > 0 && !cells.is_empty() && cells.len() <= MAX_TABLE_COLUMNS).then_some(cells)
}

fn table_header<'a>(line: &'a str, separator: &str) -> Option<(Vec<&'a str>, Vec<Align>)> {
    if !separator.contains('|') {
        return None;
    }
    let header = table_cells(line)?;
    let delimiters = table_cells(separator)?;
    if header.len() != delimiters.len() {
        return None;
    }
    let alignments = delimiters
        .iter()
        .map(|cell| {
            let dashes = cell.strip_prefix(':').unwrap_or(cell);
            let dashes = dashes.strip_suffix(':').unwrap_or(dashes);
            if dashes.is_empty() || !dashes.bytes().all(|ch| ch == b'-') {
                return None;
            }
            Some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (_, true) => Align::Max,
                _ => Align::Min,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((header, alignments))
}

fn unescape_table_pipes(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut start = 0;
    let mut slashes = 0;
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'|' && slashes % 2 == 1 {
            out.push_str(&text[start..offset - 1]);
            start = offset;
        }
        slashes = if byte == b'\\' { slashes + 1 } else { 0 };
    }
    if start == 0 {
        Cow::Borrowed(text)
    } else {
        out.push_str(&text[start..]);
        Cow::Owned(out)
    }
}

fn paragraph<'a>(first: &'a str, lines: &mut Lines<'a>, indent: usize) -> Cow<'a, str> {
    let mut body = Cow::Borrowed(first);
    while let Some((_, next)) = lines.peek() {
        let next = next.trim_end_matches('\r');
        let (next_indent, trimmed) = indentation(next);
        if trimmed.is_empty()
            || next_indent < indent
            || list_item(next).is_some()
            || trimmed.starts_with("```")
            || trimmed.starts_with('|')
            || heading(trimmed).is_some()
        {
            break;
        }
        // Soft source line breaks belong to the same item, not a new bullet.
        // Explicit two-space hard breaks keep their newline.
        let separator = if body.ends_with("  ") { '\n' } else { ' ' };
        let body = body.to_mut();
        body.truncate(body.trim_end().len());
        body.push(separator);
        body.push_str(trimmed);
        lines.next();
    }
    body
}

pub(super) fn show(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    let mut lines = text.split('\n').enumerate().peekable();
    let mut code: Option<(String, String)> = None;
    let mut lists: Vec<ListLevel> = Vec::new();
    while let Some((index, line)) = lines.next() {
        let line = line.trim_end_matches('\r');
        if let Some(language) = line.strip_prefix("```") {
            lists.clear();
            if let Some((language, body)) = code.take() {
                code_block(ui, &language, &body);
            } else {
                code = Some((language.to_owned(), String::new()));
            }
        } else if let Some((_, body)) = &mut code {
            body.push_str(line);
            body.push('\n');
        } else if line.trim().is_empty() {
            ui.add_space(5.0);
        } else if let Some(heading) = heading(line) {
            lists.clear();
            ui.add_space(8.0);
            inline_label(ui, heading, size + 2.0, TEXT, true);
        } else if let Some((header, alignments)) =
            lines.peek().and_then(|(_, next)| table_header(line, next))
        {
            lists.clear();
            lines.next(); // Confirmed delimiter row: not visible table content.
            ui.push_id(("markdown_table", index), |ui| {
                table(ui, &header, &mut lines, &alignments, size, color)
            });
        } else if let Some(item) = list_item(line) {
            while lists.last().is_some_and(|level| level.indent > item.indent) {
                lists.pop();
            }
            let previous = lists.last().filter(|level| {
                level.indent == item.indent
                    && level.next_number.is_some() == item.number.is_some()
                    && level.delimiter == item.delimiter
            });
            let number = previous.and_then(|level| level.next_number).or(item.number);
            if lists
                .last()
                .is_some_and(|level| level.indent == item.indent)
            {
                lists.pop();
            }
            let marker = number.map_or_else(
                || "•".to_owned(),
                |number| format!("{number}{}", item.delimiter),
            );
            let body = paragraph(item.body, &mut lines, item.indent);
            let offset = ui
                .push_id(("markdown_list", index), |ui| {
                    list_row(ui, &marker, &body, item.indent, size, color)
                })
                .inner;
            lists.push(ListLevel {
                indent: item.indent,
                content_indent: item.content_indent,
                text_offset: offset,
                next_number: number.map(|number| number.saturating_add(1)),
                delimiter: item.delimiter,
            });
        } else {
            let (indent, trimmed) = indentation(line);
            if let Some(level) = lists
                .iter()
                .rev()
                .find(|level| indent >= level.content_indent)
            {
                let offset = level.text_offset;
                let body = paragraph(trimmed, &mut lines, level.indent);
                ui.push_id(("markdown_list_continuation", index), |ui| {
                    list_body(ui, offset, &body, size, color)
                });
            } else {
                lists.clear();
                inline_label(ui, line, size, color, false);
            }
        }
    }
    if let Some((language, body)) = code {
        code_block(ui, &language, &body);
    }
}

fn list_row(
    ui: &mut Ui,
    marker: &str,
    body: &str,
    indent: usize,
    size: f32,
    color: Color32,
) -> f32 {
    let origin = ui.cursor().min;
    let width = ui.available_width();
    let indent = (indent as f32 * size * 0.6).min(width * 0.5);
    let marker_width = ui
        .painter()
        .layout_no_wrap(marker.into(), FontId::proportional(size), color)
        .size()
        .x;
    let gutter = (size * 1.6).max(marker_width + 8.0);
    let offset = (indent + gutter).min((width - 1.0).max(0.0));
    let mut marker_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("marker")
            .max_rect(Rect::from_min_max(
                origin + vec2(indent, 0.0),
                pos2(origin.x + (offset - 6.0).max(indent + 1.0), f32::INFINITY),
            ))
            .layout(Layout::top_down(Align::Max)),
    );
    inline_label(&mut marker_ui, marker, size, color, false);
    let rect = list_body(ui, offset, body, size, color);
    if marker_ui.min_rect().bottom() > rect.bottom() {
        ui.advance_cursor_after_rect(Rect::from_min_max(
            origin,
            pos2(rect.right(), marker_ui.min_rect().bottom()),
        ));
    }
    offset
}

fn list_body(ui: &mut Ui, offset: f32, body: &str, size: f32, color: Color32) -> Rect {
    let origin = ui.cursor().min;
    let right = ui.max_rect().right();
    let mut body_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("body")
            .max_rect(Rect::from_min_max(
                origin + vec2(offset, 0.0),
                pos2(right, f32::INFINITY),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    inline_label(&mut body_ui, body, size, color, false);
    let rect = Rect::from_min_max(origin, pos2(right, body_ui.min_rect().bottom()));
    ui.advance_cursor_after_rect(rect);
    rect
}

fn table(
    ui: &mut Ui,
    header: &[&str],
    lines: &mut Lines<'_>,
    alignments: &[Align],
    size: f32,
    color: Color32,
) {
    // Equal, fixed columns avoid per-frame width jitter while streaming. Narrow
    // cells wrap; genuinely wide tables scroll within the conversation width.
    let width = ui
        .available_width()
        .max(header.len() as f32 * (size * 6.0).max(18.0));
    egui::ScrollArea::horizontal()
        .id_salt("table_scroll")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let origin = ui.cursor().min;
            ui.push_id(("row", 0), |ui| {
                table_row(ui, header, alignments, width, size, color, 0)
            });
            let mut index = 1;
            while let Some((_, next)) = lines.peek() {
                if next.trim_start().starts_with("```")
                    || list_item(next).is_some()
                    || heading(next.trim_start()).is_some()
                {
                    break;
                }
                let Some(cells) = table_cells(next) else {
                    break;
                };
                if cells.len() > header.len() {
                    break;
                }
                ui.push_id(("row", index), |ui| {
                    table_row(ui, &cells, alignments, width, size, color, index)
                });
                lines.next();
                index += 1;
            }
            let rect = Rect::from_min_max(origin, pos2(origin.x + width, ui.cursor().min.y));
            ui.painter()
                .rect_stroke(rect, 4, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
            let column = width / header.len() as f32;
            for index in 1..header.len() {
                ui.painter().vline(
                    origin.x + column * index as f32,
                    rect.y_range(),
                    Stroke::new(1.0, LINE),
                );
            }
        });
}

fn table_row(
    ui: &mut Ui,
    cells: &[&str],
    alignments: &[Align],
    width: f32,
    size: f32,
    color: Color32,
    index: usize,
) {
    let origin = ui.cursor().min;
    let column = width / alignments.len() as f32;
    let padding = 8.0;
    let background = ui.painter().add(egui::Shape::Noop);
    let mut height = 0.0_f32;
    let header = index == 0;
    for (index, align) in alignments.iter().enumerate() {
        let left = origin.x + column * index as f32;
        let mut cell_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(("cell", index))
                .max_rect(Rect::from_min_max(
                    pos2(left + padding, origin.y + padding),
                    pos2(left + column - padding, f32::INFINITY),
                ))
                .layout(Layout::top_down(*align)),
        );
        let text = unescape_table_pipes(cells.get(index).copied().unwrap_or(""));
        // Header uses the same cached inline/link renderer as the body.
        inline_label(
            &mut cell_ui,
            &text,
            size,
            if header { TEXT } else { color },
            header,
        );
        height = height.max(cell_ui.min_rect().bottom() - origin.y + padding);
    }
    let rect = Rect::from_min_size(origin, vec2(width, height));
    let fill = if index == 0 {
        SURFACE
    } else if index.is_multiple_of(2) {
        RAIL
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .set(background, egui::Shape::rect_filled(rect, 0, fill));
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, LINE));
    ui.advance_cursor_after_rect(rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_markers_require_separating_whitespace_and_reject_literal_syntax() {
        for source in [
            "- item",
            "+ item",
            "* item",
            "  - nested",
            "17) numbered",
            "2. numbered",
            "-\titem",
        ] {
            assert!(list_item(source).is_some(), "{source}");
        }
        for source in [
            "-",
            "*bold*",
            "-12",
            "1.23",
            "1234567890. too long",
            "---",
            "* * *",
            r"\- literal",
        ] {
            assert!(list_item(source).is_none(), "{source}");
        }
        let item = list_item("\t17) café 🌿").unwrap();
        assert_eq!(item.indent, 4);
        assert_eq!(item.content_indent, 8);
        assert_eq!(item.number, Some(17));
        assert_eq!(item.body, "café 🌿");
        assert_eq!(list_item("-\titem").unwrap().content_indent, 4);
    }

    #[test]
    fn pipe_rows_support_optional_edges_escapes_code_unicode_and_empty_cells() {
        assert_eq!(table_cells("A | B").unwrap(), ["A", "B"]);
        assert_eq!(table_cells(" | café | 🌿 | ").unwrap(), ["café", "🌿"]);
        assert_eq!(
            table_cells(r"| `a\|b` | x\|y | `a|b` |").unwrap(),
            [r"`a\|b`", r"x\|y", "`a|b`"]
        );
        assert_eq!(table_cells(r"A \\| B").unwrap(), [r"A \\", "B"]);
        assert_eq!(table_cells("| | B | |").unwrap(), ["", "B", ""]);
        assert!(table_cells("plain text").is_none());
        assert_eq!(unescape_table_pipes(r"`a\|b`"), "`a|b`");
        assert_eq!(unescape_table_pipes(r"a\\|b"), r"a\\|b");
    }

    #[test]
    fn tables_require_a_matching_valid_separator_and_have_bounded_columns() {
        assert_eq!(
            table_header("A | B | C", ":--- | :---: | ---:").unwrap().1,
            [Align::Min, Align::Center, Align::Max]
        );
        for separator in [
            "| --- |",
            "| --- | words |",
            "| ::--- | --- |",
            "| : | --- |",
            "| --- | --",
        ] {
            assert!(
                table_header("A | B | C", separator).is_none(),
                "{separator}"
            );
        }
        let header = "cell | ".repeat(MAX_TABLE_COLUMNS + 1);
        assert!(table_cells(&header).is_none());
        assert!(table_header(&header, &"--- | ".repeat(MAX_TABLE_COLUMNS + 1)).is_none());
    }
}
