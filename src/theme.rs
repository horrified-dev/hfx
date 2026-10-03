use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2,
};

// Layered ink-blue darks: rail < sidebar < background < surface < raised < hover.
pub const BG: Color32 = Color32::from_rgb(19, 19, 24);
pub const SIDEBAR: Color32 = Color32::from_rgb(15, 15, 19);
pub const RAIL: Color32 = Color32::from_rgb(11, 11, 14);
pub const SURFACE: Color32 = Color32::from_rgb(28, 28, 36);
pub const RAISED: Color32 = Color32::from_rgb(35, 35, 46);
/// Resting fill for cards and inset panels (sits between BG and SURFACE).
pub const CARD: Color32 = Color32::from_rgb(23, 23, 30);
/// Tinted fill and border for reasoning/thinking blocks.
pub const THINK_BG: Color32 = Color32::from_rgb(23, 23, 32);
pub const THINK_LINE: Color32 = Color32::from_rgb(42, 44, 70);
pub const HOVER: Color32 = Color32::from_rgb(41, 41, 54);
pub const LINE: Color32 = Color32::from_rgb(39, 39, 51);
pub const OUTLINE: Color32 = Color32::from_rgb(62, 62, 82);
pub const TEXT: Color32 = Color32::from_rgb(236, 236, 242);
pub const MUTED: Color32 = Color32::from_rgb(166, 166, 186);
pub const DIM: Color32 = Color32::from_rgb(128, 128, 150);
pub const ACCENT: Color32 = Color32::from_rgb(150, 160, 255);
pub const CODE: Color32 = Color32::from_rgb(210, 214, 240);
pub const ERROR: Color32 = Color32::from_rgb(242, 135, 135);

pub fn install(ctx: &egui::Context, font_size: f32, reduced: bool) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = SIDEBAR;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(54, 60, 112);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.window_stroke = Stroke::new(1.0, OUTLINE);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.menu_corner_radius = 12.into();
    let shadow = |blur: u8, alpha: u8| egui::epaint::Shadow {
        offset: [0, 8],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    };
    style.visuals.popup_shadow = shadow(24, 140);
    style.visuals.window_shadow = shadow(32, 160);
    style.visuals.window_corner_radius = 14.into();
    for visuals in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.noninteractive,
    ] {
        visuals.corner_radius = 9.into();
        visuals.bg_stroke = Stroke::new(1.0, LINE);
        visuals.fg_stroke = Stroke::new(1.0, TEXT);
    }
    style.visuals.widgets.inactive.bg_fill = RAISED;
    style.visuals.widgets.inactive.weak_bg_fill = RAISED;
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, MUTED);
    style.visuals.widgets.hovered.bg_fill = HOVER;
    style.visuals.widgets.hovered.weak_bg_fill = HOVER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, OUTLINE);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(46, 50, 94);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(46, 50, 94);
    style.visuals.widgets.noninteractive.bg_fill = SIDEBAR;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(10.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.animation_time = if reduced { 0.0 } else { 0.22 };
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(font_size));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

#[derive(Clone, Copy)]
pub enum Icon {
    Home,
    Folder,
    Plus,
    Search,
    Clock,
    Settings,
    Sidebar,
    Terminal,
    Arrow,
    Queue,
    Close,
    Check,
    Cpu,
    Globe,
    Spark,
    Stop,
    Copy,
    Minus,
    Maximize,
    Paperclip,
    Pencil,
    Trash,
}

pub fn icon(p: &egui::Painter, center: Pos2, size: f32, kind: Icon, color: Color32) {
    let s = size / 20.0;
    let xy = |x: f32, y: f32| center + vec2(x * s, y * s);
    let stroke = Stroke::new(1.35 * s.max(0.85), color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([xy(a.0, a.1), xy(b.0, b.1)], stroke);
    };
    let rect = |x: f32, y: f32, w: f32, h: f32, r: u8| {
        p.rect_stroke(
            Rect::from_min_size(xy(x, y), vec2(w * s, h * s)),
            r,
            stroke,
            egui::StrokeKind::Middle,
        );
    };
    match kind {
        Icon::Home => {
            line((-8.0, -1.0), (0.0, -8.0));
            line((0.0, -8.0), (8.0, -1.0));
            line((-6.0, -2.0), (-6.0, 7.0));
            line((-6.0, 7.0), (6.0, 7.0));
            line((6.0, 7.0), (6.0, -2.0));
            rect(-2.0, 1.0, 4.0, 6.0, 1);
        }
        Icon::Folder => {
            p.add(egui::Shape::closed_line(
                vec![
                    xy(-8.0, -5.0),
                    xy(-2.0, -5.0),
                    xy(0.0, -2.0),
                    xy(8.0, -2.0),
                    xy(6.0, 6.0),
                    xy(-8.0, 6.0),
                ],
                stroke,
            ));
            line((-8.0, -1.0), (7.0, -1.0));
        }
        Icon::Plus => {
            line((-6.0, 0.0), (6.0, 0.0));
            line((0.0, -6.0), (0.0, 6.0));
        }
        Icon::Search => {
            p.circle_stroke(xy(-1.5, -1.5), 5.5 * s, stroke);
            line((3.0, 3.0), (7.0, 7.0));
        }
        Icon::Clock => {
            p.circle_stroke(center, 8.0 * s, stroke);
            line((0.0, -4.0), (0.0, 0.0));
            line((0.0, 0.0), (4.0, 2.0));
        }
        Icon::Settings => {
            p.circle_stroke(center, 5.8 * s, stroke);
            p.circle_stroke(center, 2.2 * s, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                p.line_segment(
                    [
                        center + vec2(a.cos(), a.sin()) * 6.0 * s,
                        center + vec2(a.cos(), a.sin()) * 8.5 * s,
                    ],
                    stroke,
                );
            }
        }
        Icon::Sidebar => {
            rect(-8.0, -7.0, 16.0, 14.0, 2);
            line((-2.0, -7.0), (-2.0, 7.0));
        }
        Icon::Terminal => {
            line((-6.0, -4.0), (-2.0, 0.0));
            line((-2.0, 0.0), (-6.0, 4.0));
            line((1.0, 4.0), (7.0, 4.0));
        }
        Icon::Arrow => {
            line((0.0, 6.0), (0.0, -6.0));
            line((0.0, -6.0), (-4.5, -1.5));
            line((0.0, -6.0), (4.5, -1.5));
        }
        Icon::Queue => {
            line((-7.0, -6.0), (-7.0, 4.0));
            line((-7.0, 4.0), (7.0, 4.0));
            line((7.0, 4.0), (3.0, 0.0));
            line((7.0, 4.0), (3.0, 8.0));
            line((-3.0, -5.0), (6.0, -5.0));
            line((-3.0, -1.0), (3.0, -1.0));
        }
        Icon::Close => {
            line((-4.0, -4.0), (4.0, 4.0));
            line((-4.0, 4.0), (4.0, -4.0));
        }
        Icon::Check => {
            line((-5.0, 0.0), (-1.0, 4.0));
            line((-1.0, 4.0), (6.0, -4.0));
        }
        Icon::Cpu => {
            rect(-5.0, -5.0, 10.0, 10.0, 2);
            rect(-2.0, -2.0, 4.0, 4.0, 0);
            for i in [-3.0, 0.0, 3.0] {
                line((i, -8.0), (i, -5.0));
                line((i, 5.0), (i, 8.0));
                line((-8.0, i), (-5.0, i));
                line((5.0, i), (8.0, i));
            }
        }
        Icon::Globe => {
            p.circle_stroke(center, 8.0 * s, stroke);
            p.line_segment([xy(-8.0, 0.0), xy(8.0, 0.0)], stroke);
            p.add(egui::Shape::ellipse_stroke(
                center,
                vec2(3.5, 8.0) * s,
                stroke,
            ));
        }
        Icon::Spark => {
            p.add(egui::Shape::closed_line(
                vec![
                    xy(0.0, -8.0),
                    xy(2.0, -2.0),
                    xy(8.0, 0.0),
                    xy(2.0, 2.0),
                    xy(0.0, 8.0),
                    xy(-2.0, 2.0),
                    xy(-8.0, 0.0),
                    xy(-2.0, -2.0),
                ],
                stroke,
            ));
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_center_size(center, vec2(9.0, 9.0) * s), 2, color);
        }
        Icon::Copy => {
            rect(-6.0, -6.0, 9.0, 10.0, 1);
            rect(-2.0, -2.0, 9.0, 10.0, 1);
        }
        Icon::Minus => line((-5.0, 0.0), (5.0, 0.0)),
        Icon::Maximize => rect(-4.5, -4.5, 9.0, 9.0, 1),
        Icon::Pencil => {
            p.add(egui::Shape::closed_line(
                vec![
                    xy(-7.0, 7.0),
                    xy(-6.0, 2.0),
                    xy(3.0, -7.0),
                    xy(7.0, -3.0),
                    xy(-2.0, 6.0),
                ],
                stroke,
            ));
            line((1.0, -5.0), (5.0, -1.0));
        }
        Icon::Trash => {
            line((-7.0, -4.0), (7.0, -4.0));
            rect(-3.0, -7.0, 6.0, 3.0, 1);
            p.add(egui::Shape::line(
                vec![xy(-5.0, -4.0), xy(-4.0, 7.0), xy(4.0, 7.0), xy(5.0, -4.0)],
                stroke,
            ));
            line((-1.5, -1.0), (-1.5, 4.0));
            line((1.5, -1.0), (1.5, 4.0));
        }
        Icon::Paperclip => {
            p.add(egui::Shape::line(
                vec![
                    xy(4.0, -4.0),
                    xy(-3.0, 3.0),
                    xy(-1.0, 5.0),
                    xy(6.0, -2.0),
                    xy(6.0, -6.0),
                    xy(2.0, -8.0),
                    xy(-6.0, 0.0),
                    xy(-6.0, 5.0),
                    xy(-2.0, 8.0),
                    xy(3.0, 6.0),
                ],
                stroke,
            ));
        }
    }
}

/// Participate in an inline row's layout so the glyph and neighboring labels
/// share its vertical center. Never guess y from the cursor before layout.
pub fn inline_icon(ui: &mut Ui, kind: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    icon(ui.painter(), rect.center(), size, kind, color);
    response
}

pub fn icon_button(ui: &mut Ui, kind: Icon, label: &str, selected: bool, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hover =
        ui.ctx()
            .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.14);
    if selected {
        ui.painter()
            .rect_filled(rect, 10, ACCENT.gamma_multiply(0.14));
    }
    if hover > 0.0 {
        ui.painter()
            .rect_filled(rect, 10, HOVER.gamma_multiply(hover));
    }
    icon(
        ui.painter(),
        rect.center(),
        18.0,
        kind,
        if selected {
            ACCENT
        } else if response.hovered() {
            TEXT
        } else {
            MUTED
        },
    );
    response.on_hover_text(label)
}

pub fn row(ui: &mut Ui, title: &str, glyph: Option<Icon>, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let hover =
        ui.ctx()
            .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.14);
    if selected {
        ui.painter().rect_filled(rect, 9, SURFACE);
        ui.painter()
            .rect_filled(rect, 9, ACCENT.gamma_multiply(0.06));
        ui.painter().rect_filled(
            Rect::from_center_size(pos2(rect.left() + 2.0, rect.center().y), vec2(3.0, 16.0)),
            2,
            ACCENT,
        );
    } else if hover > 0.0 {
        ui.painter()
            .rect_filled(rect, 9, SURFACE.gamma_multiply(hover));
    }
    let x = if let Some(glyph) = glyph {
        icon(
            ui.painter(),
            pos2(rect.left() + 17.0, rect.center().y),
            16.0,
            glyph,
            if selected { ACCENT } else { MUTED },
        );
        rect.left() + 34.0
    } else {
        rect.left() + 12.0
    };
    let painter = ui.painter().with_clip_rect(rect.shrink2(vec2(10.0, 0.0)));
    painter.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        if selected { TEXT } else { MUTED },
    );
    response
}

/// A compact, inset frame for the chat context menu (not other app popups).
pub fn chat_menu_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::popup(style)
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(6)
}

/// Native button interaction/accessibility, with a hand-drawn leading icon.
pub fn menu_action(ui: &mut Ui, label: &str, glyph: Icon, destructive: bool) -> Response {
    ui.scope(|ui| {
        let color = if destructive { ERROR } else { TEXT };
        let style = ui.style_mut();
        style.visuals.override_text_color = Some(color);
        style.spacing.button_padding = vec2(10.0, 7.0);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        for visuals in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            visuals.corner_radius = 6.into();
            visuals.bg_stroke = Stroke::NONE;
            visuals.fg_stroke = Stroke::new(1.0, color);
        }
        style.visuals.widgets.hovered.weak_bg_fill = if destructive {
            ERROR.gamma_multiply(0.12)
        } else {
            HOVER
        };
        style.visuals.widgets.active.weak_bg_fill = if destructive {
            ERROR.gamma_multiply(0.20)
        } else {
            ACCENT.gamma_multiply(0.16)
        };
        let icon_id = ui.id().with(("menu_icon", label));
        let button = egui::Button::new((
            egui::Atom::custom(icon_id, vec2(16.0, 16.0)),
            egui::RichText::new(label).size(13.0),
            egui::Atom::grow(),
        ))
        .min_size(vec2(176.0, 34.0))
        .gap(10.0)
        .atom_ui(ui);
        if let Some(rect) = button.rect(icon_id) {
            icon(
                ui.painter(),
                rect.center(),
                16.0,
                glyph,
                if !ui.is_enabled() {
                    DIM
                } else if destructive {
                    ERROR
                } else {
                    MUTED
                },
            );
        }
        button.response
    })
    .inner
}

pub fn menu_divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
    ui.painter().line_segment(
        [
            pos2(rect.left() + 8.0, rect.center().y),
            pos2(rect.right() - 8.0, rect.center().y),
        ],
        Stroke::new(1.0, LINE),
    );
}

/// Restrained dialog actions with a clearly differentiated primary button.
pub fn dialog_action(ui: &mut Ui, label: &str, primary: bool) -> Response {
    ui.scope(|ui| {
        let style = ui.style_mut();
        style.visuals.override_text_color = Some(if primary { BG } else { TEXT });
        for (visuals, fill) in [
            (
                &mut style.visuals.widgets.inactive,
                if primary { ACCENT } else { RAISED },
            ),
            (
                &mut style.visuals.widgets.hovered,
                if primary {
                    Color32::from_rgb(175, 183, 255)
                } else {
                    HOVER
                },
            ),
            (
                &mut style.visuals.widgets.active,
                if primary {
                    Color32::from_rgb(132, 143, 240)
                } else {
                    SURFACE
                },
            ),
        ] {
            visuals.weak_bg_fill = fill;
            visuals.bg_fill = fill;
            visuals.corner_radius = 8.into();
            visuals.bg_stroke = if primary {
                Stroke::NONE
            } else {
                Stroke::new(1.0, LINE)
            };
        }
        ui.add(egui::Button::new(egui::RichText::new(label).size(13.0)).min_size(vec2(84.0, 36.0)))
    })
    .inner
}

pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.28)))
        .corner_radius(99)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(10.0).color(color));
        });
}

pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(16.0);
    ui.label(egui::RichText::new(text).size(11.0).color(MUTED).strong());
    ui.add_space(3.0);
}

pub fn logo(ui: &mut Ui, size: f32, time: f64, animated: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let c = rect.center();
    let breath = if animated {
        (time as f32 * 1.4).sin() * 0.025
    } else {
        0.0
    };
    let r = size * (0.38 + breath);
    let points = (0..=64)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 64.0;
            let radius = r * (1.0 + 0.075 * (a * 6.0).cos());
            c + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    ui.painter().add(egui::Shape::line(
        points,
        Stroke::new(1.5, ACCENT.gamma_multiply(0.65)),
    ));
    icon(ui.painter(), c, size * 0.4, Icon::Terminal, ACCENT);
}

pub fn rich_inline(text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut rest = text;
    while !rest.is_empty() {
        let next = rest
            .find("**")
            .map(|i| (i, "**"))
            .into_iter()
            .chain(rest.find('`').map(|i| (i, "`")))
            .min_by_key(|(i, _)| *i);
        let Some((start, marker)) = next else {
            job.append(
                rest,
                0.0,
                egui::TextFormat {
                    font_id: FontId::proportional(size),
                    color,
                    ..Default::default()
                },
            );
            break;
        };
        job.append(
            &rest[..start],
            0.0,
            egui::TextFormat {
                font_id: FontId::proportional(size),
                color,
                ..Default::default()
            },
        );
        let after = &rest[start + marker.len()..];
        if let Some(end) = after.find(marker) {
            job.append(
                &after[..end],
                0.0,
                egui::TextFormat {
                    font_id: if marker == "`" {
                        FontId::monospace(size - 1.0)
                    } else {
                        FontId::proportional(size)
                    },
                    color: if marker == "`" { ACCENT } else { TEXT },
                    background: if marker == "`" {
                        SURFACE
                    } else {
                        Color32::TRANSPARENT
                    },
                    extra_letter_spacing: if marker == "**" { 0.2 } else { 0.0 },
                    ..Default::default()
                },
            );
            rest = &after[end + marker.len()..];
        } else {
            job.append(
                &rest[start..],
                0.0,
                egui::TextFormat {
                    font_id: FontId::proportional(size),
                    color,
                    ..Default::default()
                },
            );
            break;
        }
    }
    job
}

/// Stable block layout while streaming. Code blocks can be copied independently.
pub fn markdown(ui: &mut Ui, text: &str, size: f32, color: Color32) {
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
        } else {
            let heading = line
                .strip_prefix("### ")
                .or_else(|| line.strip_prefix("## "))
                .or_else(|| line.strip_prefix("# "));
            if let Some(heading) = heading {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(heading)
                        .size(size + 2.0)
                        .strong()
                        .color(TEXT),
                );
            } else {
                ui.add(
                    egui::Label::new(rich_inline(line, size, color))
                        .wrap()
                        .selectable(true),
                );
            }
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
                    if icon_button(ui, Icon::Copy, "Copy code", false, 24.0).clicked() {
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

    #[test]
    fn inline_icons_share_label_centers_across_row_heights_and_display_scales() {
        for pixels_per_point in [1.0, 1.5, 2.0] {
            for row_height in [24.0, 30.0, 40.0] {
                let ctx = egui::Context::default();
                install(&ctx, 14.0, true);
                ctx.set_pixels_per_point(pixels_per_point);
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.spacing_mut().interact_size.y = row_height;
                    for (kind, icon_size, label_size) in [(Icon::Terminal, 16.0, 13.0), (Icon::Folder, 13.0, 11.0), (Icon::Folder, 15.0, 14.0)] {
                        ui.horizontal(|ui| {
                            let icon = inline_icon(ui, kind, icon_size, ACCENT);
                            let label = ui.label(egui::RichText::new("hfx").size(label_size));
                            assert!((icon.rect.center().y - label.rect.center().y).abs() < 0.05,
                                "icon/label center mismatch at scale {pixels_per_point}, height {row_height}");
                            assert!((label.rect.left() - icon.rect.right() - ui.spacing().item_spacing.x).abs() < 0.05);
                        });
                    }
                });
                output.textures_delta.clear();
            }
        }
    }
}
