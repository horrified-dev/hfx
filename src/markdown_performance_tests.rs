//! Opt-in comparisons using the original owned parser/render path and the cache.
use super::*;

#[test]
#[ignore = "manual distinct-line Markdown rendering comparison"]
fn profile_distinct_markdown_lines() {
    use std::{hint::black_box, time::Instant};
    for count in [128, 512, 2_048, 8_192] {
        let lines: Vec<String> = (0..count).map(|index| format!(
            "Distinct line {index}: **formatted prose**, Unicode café 🌿 and a [documentation link](https://example.com/docs/{index})."
        )).collect();
        for cached in [false, true] {
            let ctx = egui::Context::default();
            theme::install(&ctx, 14.0, true);
            let render = |frame| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1180.0, 820.0),
                        )),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_max_width(720.0);
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for line in &lines {
                                    if cached {
                                        inline_label(ui, line, 14.0, TEXT, false);
                                    } else {
                                        // Exact legacy behavior: parse an owned job and
                                        // move it to the label, without a cache/copy.
                                        let inline = inline_text(line, 14.0, TEXT, false);
                                        paint_inline(ui, inline.job, &inline.links, TEXT);
                                    }
                                }
                            });
                    },
                );
                output.textures_delta.clear();
                black_box(output);
            };
            for frame in 0..10 {
                render(frame);
            }
            let frames = 30;
            let start = Instant::now();
            for frame in 10..10 + frames {
                render(frame);
            }
            println!(
                "{count} distinct Markdown lines cached={cached}: {:.3} ms/frame",
                start.elapsed().as_secs_f64() * 1000.0 / frames as f64
            );
        }
    }
}
