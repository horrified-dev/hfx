//! Shared presentation for the settings drawer. Controls retain native egui
//! interaction, focus, and accessibility; state and side effects live in app.
use crate::{
    state::Provider,
    theme::{self, Icon},
};
use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Margin, Response, RichText, Sense, Stroke, Ui, pos2, vec2,
};

pub const GREEN: Color32 = Color32::from_rgb(143, 202, 175);
pub const AMBER: Color32 = Color32::from_rgb(224, 186, 129);
const SELECTED: Color32 = Color32::from_rgb(34, 35, 55);

pub fn style(ui: &mut Ui) {
    let style = ui.style_mut();
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(10.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.visuals.text_edit_bg_color = Some(theme::SIDEBAR);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, theme::LINE);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme::LINE);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
}

pub fn intro(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(title).size(19.0).strong().color(theme::TEXT));
    ui.label(RichText::new(subtitle).size(12.0).color(theme::MUTED));
    ui.add_space(8.0);
}

pub fn card<R>(ui: &mut Ui, id: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    let result = ui
        .push_id(id, |ui| {
            Frame::NONE
                .fill(theme::CARD)
                .stroke(Stroke::new(1.0, theme::LINE))
                .corner_radius(12)
                .inner_margin(15)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 3],
                    blur: 10,
                    spread: 0,
                    color: Color32::from_black_alpha(45),
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui)
                })
                .inner
        })
        .inner;
    ui.add_space(6.0);
    result
}

pub fn heading(ui: &mut Ui, glyph: Icon, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
        ui.painter().rect_filled(rect, 8, theme::RAISED);
        theme::icon(ui.painter(), rect.center(), 16.0, glyph, theme::ACCENT);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(13.0).strong().color(theme::TEXT));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(11.0).color(theme::DIM));
            }
        });
    });
    ui.add_space(5.0);
}

pub fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(theme::MUTED));
}

pub fn help(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.0).color(theme::DIM)).wrap());
}

pub fn divider(ui: &mut Ui) {
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(3.0);
}

pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    Frame::NONE
        .fill(color.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.22)))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(7, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.0).strong().color(color));
        });
}

pub fn callout(ui: &mut Ui, text: &str, color: Color32) {
    Frame::NONE
        .fill(color.gamma_multiply(0.06))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.20)))
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).wrap());
        });
}

pub fn field(ui: &mut Ui, value: &mut String, hint: &str, secret: bool) -> Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .password(secret)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(vec2(10.0, 9.0)),
    )
}

/// One focusable checkbox response for the complete row, painted as a switch.
/// This preserves Space/keyboard activation and reports the real checked state.
pub fn toggle(ui: &mut Ui, value: &mut bool, title: &str, subtitle: &str) -> Response {
    let width = ui.available_width();
    let text_width = (width - 56.0).max(30.0);
    let title_galley = ui.painter().layout(
        title.into(),
        FontId::proportional(13.0),
        theme::TEXT,
        text_width,
    );
    let subtitle_galley = ui.painter().layout(
        subtitle.into(),
        FontId::proportional(11.0),
        theme::DIM,
        text_width,
    );
    let text_height = title_galley.size().y
        + if subtitle.is_empty() {
            0.0
        } else {
            4.0 + subtitle_galley.size().y
        };
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(width, text_height.max(28.0) + 4.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, title)
    });
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left(), rect.center().y - text_height / 2.0),
        title_galley,
        theme::TEXT,
    );
    if !subtitle.is_empty() {
        painter.galley(
            pos2(
                rect.left(),
                rect.center().y + text_height / 2.0 - subtitle_galley.size().y,
            ),
            subtitle_galley,
            theme::DIM,
        );
    }
    let switch = egui::Rect::from_center_size(
        pos2(rect.right() - 19.0, rect.top() + 14.0),
        vec2(38.0, 22.0),
    );
    let fill = if *value {
        theme::ACCENT
    } else if response.hovered() {
        theme::HOVER
    } else {
        theme::RAISED
    };
    painter.rect_filled(switch, 11, fill);
    painter.rect_stroke(
        switch,
        11,
        Stroke::new(
            1.0,
            if response.has_focus() {
                theme::TEXT
            } else {
                theme::OUTLINE
            },
        ),
        egui::StrokeKind::Inside,
    );
    let x = if *value {
        switch.right() - 11.0
    } else {
        switch.left() + 11.0
    };
    painter.circle_filled(
        pos2(x, switch.center().y + 1.0),
        7.5,
        Color32::from_black_alpha(45),
    );
    painter.circle_filled(
        pos2(x, switch.center().y),
        7.0,
        if *value { theme::BG } else { theme::MUTED },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn tabs(ui: &mut Ui, selected: &mut usize) {
    Frame::NONE
        .fill(theme::RAIL)
        .stroke(Stroke::new(1.0, theme::LINE))
        .corner_radius(10)
        .inner_margin(4)
        .show(ui, |ui| {
            let width = (ui.available_width() - 8.0) / 3.0;
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.spacing_mut().button_padding = vec2(5.0, 6.0);
            ui.horizontal(|ui| {
                for (index, (title, glyph)) in [
                    ("Providers", Icon::Cpu),
                    ("Appearance", Icon::Spark),
                    ("Tools", Icon::Terminal),
                ]
                .into_iter()
                .enumerate()
                {
                    let current = *selected == index;
                    let icon_id = ui.id().with(("tab_icon", index));
                    let button = egui::Button::new((
                        egui::Atom::custom(icon_id, vec2(13.0, 13.0)),
                        RichText::new(title).size(12.0).color(if current {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        }),
                    ))
                    .min_size(vec2(width, 36.0))
                    .gap(5.0)
                    .fill(if current {
                        SELECTED
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if current {
                            theme::OUTLINE
                        } else {
                            Color32::TRANSPARENT
                        },
                    ))
                    .corner_radius(7)
                    .selected(current)
                    .atom_ui(ui);
                    if let Some(rect) = button.rect(icon_id) {
                        theme::icon(
                            ui.painter(),
                            rect.center(),
                            13.0,
                            glyph,
                            if current { theme::ACCENT } else { theme::DIM },
                        );
                    }
                    if button.response.has_focus() {
                        ui.painter().rect_stroke(
                            button.response.rect,
                            7,
                            Stroke::new(1.0, theme::ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if button.response.clicked() {
                        *selected = index;
                    }
                }
            });
        });
}

pub fn provider(ui: &mut Ui, provider: Provider, selected: bool, width: f32) -> Response {
    let (subtitle, glyph) = match provider {
        Provider::Codex => ("ChatGPT account", Icon::Spark),
        Provider::OpenAI => ("Direct API access", Icon::Globe),
        Provider::OpenRouter => ("Model marketplace", Icon::Globe),
        Provider::Llama => ("Local inference", Icon::Cpu),
        Provider::Demo => ("No account needed", Icon::Spark),
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 70.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            ui.is_enabled(),
            selected,
            provider.label(),
        )
    });
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect.translate(vec2(0.0, 2.0)), 10, theme::RAIL);
    painter.rect_filled(
        rect,
        10,
        if selected {
            SELECTED
        } else if response.hovered() {
            theme::SURFACE
        } else {
            theme::CARD
        },
    );
    painter.rect_stroke(
        rect,
        10,
        Stroke::new(
            1.0,
            if selected || response.has_focus() {
                theme::ACCENT.gamma_multiply(0.7)
            } else if response.hovered() {
                theme::OUTLINE
            } else {
                theme::LINE
            },
        ),
        egui::StrokeKind::Inside,
    );
    theme::icon(
        &painter,
        rect.min + vec2(20.0, 23.0),
        17.0,
        glyph,
        if selected {
            theme::ACCENT
        } else {
            theme::MUTED
        },
    );
    painter.text(
        rect.min + vec2(35.0, 23.0),
        Align2::LEFT_CENTER,
        provider.label(),
        FontId::proportional(12.0),
        theme::TEXT,
    );
    painter.text(
        rect.min + vec2(12.0, 49.0),
        Align2::LEFT_CENTER,
        subtitle,
        FontId::proportional(10.0),
        theme::DIM,
    );
    if selected {
        painter.circle_filled(
            pos2(rect.right() - 13.0, rect.bottom() - 20.0),
            3.0,
            theme::ACCENT,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn effort(ui: &mut Ui, effort: &mut String) {
    label(ui, "Reasoning effort");
    let width = (ui.available_width() - 12.0) / 4.0;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.horizontal(|ui| {
            for (value, title) in [
                ("low", "Low"),
                ("medium", "Medium"),
                ("high", "High"),
                ("xhigh", "Extra"),
            ] {
                let response = ui.add_sized(
                    vec2(width, 30.0),
                    egui::Button::new(RichText::new(title).size(11.0)).selected(effort == value),
                );
                if response.clicked() {
                    *effort = value.into();
                }
                if value == "xhigh" {
                    response.on_hover_text("Extra high (xhigh); support depends on the model");
                }
            }
        });
    });
}

pub fn primary(ui: &mut Ui, title: &str) -> Response {
    // Reserve only the control's height, not all remaining panel space.
    ui.horizontal(|ui| theme::dialog_action(ui, title, true))
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_actions_use_button_height_not_the_remaining_panel_height() {
        for height in [300.0, 700.0, 2000.0] {
            let ctx = egui::Context::default();
            theme::install(&ctx, 15.0, true);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(320.0, height),
                    )),
                    ..Default::default()
                },
                |ui| {
                    style(ui);
                    let before = ui.label("Above the action");
                    let action = primary(ui, "Test connection");
                    let after = ui.label("Below the action");
                    assert!(
                        action.rect.top() - before.rect.bottom() <= 16.0,
                        "action is vertically centered in the unused panel space: {:?}",
                        action.rect
                    );
                    assert!(after.rect.top() - action.rect.bottom() <= 16.0);
                    assert!((action.rect.height() - 36.0).abs() < 1.0);
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn switches_support_keyboard_pointer_and_disabled_state() {
        let ctx = egui::Context::default();
        theme::install(&ctx, 15.0, true);
        let mut checked = false;
        let mut frame = 0;
        let mut render = |checked: &mut bool, enabled, events| {
            frame += 1;
            let mut response = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(320.0, 200.0),
                    )),
                    time: Some(frame as f64 * 0.1),
                    events,
                    ..Default::default()
                },
                |ui| {
                    response = Some(
                        ui.add_enabled_ui(enabled, |ui| {
                            toggle(
                                ui,
                                checked,
                                "Enable tools",
                                "A wrapped description that still leaves room for the switch.",
                            )
                        })
                        .inner,
                    );
                },
            );
            output.textures_delta.clear();
            response.unwrap()
        };
        let first = render(&mut checked, true, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(first.id));
        let space = egui::Event::Key {
            key: egui::Key::Space,
            physical_key: Some(egui::Key::Space),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let response = render(&mut checked, true, vec![space.clone()]);
        assert!(checked && response.changed());
        assert!(response.has_focus());
        let response = render(&mut checked, false, vec![space]);
        assert!(checked && !response.changed());
        // Hit testing uses the prior frame: register the re-enabled row first.
        render(&mut checked, true, vec![]);
        let pos = first.rect.left_center() + vec2(10.0, 0.0);
        for pressed in [true, false] {
            let response = render(
                &mut checked,
                true,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            assert_eq!(response.changed(), !pressed);
        }
        assert!(!checked, "the full label row should be clickable");
    }

    #[test]
    fn provider_cards_support_keyboard_selection() {
        let ctx = egui::Context::default();
        theme::install(&ctx, 15.0, true);
        let mut selected = false;
        let mut id = None;
        for frame in 0..2 {
            let events = if frame == 1 {
                ctx.memory_mut(|memory| memory.request_focus(id.unwrap()));
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: Some(egui::Key::Enter),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                vec![]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = provider(ui, Provider::Codex, selected, 154.0);
                    id = Some(response.id);
                    if response.clicked() {
                        selected = true;
                    }
                },
            );
            output.textures_delta.clear();
        }
        assert!(selected);
    }
}
