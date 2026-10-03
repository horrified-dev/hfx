# hfx paste integration

Vendored from egui-winit 0.36.2 (MIT OR Apache-2.0; distributed here under MIT).
The sole behavior change is in State::on_keyboard_input: every paste shortcut
emits Event::Paste, including an empty string when the clipboard has no text.
This lets hfx's raw_input_hook read clipboard images and file lists. Upstream
consumes the keyboard shortcut without emitting an event for these formats.
Ordinary text pasting is unchanged. Recheck this patch when updating egui.
