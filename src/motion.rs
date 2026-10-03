/// A UTF-8-safe reveal cursor. Network chunks can arrive in bursts; the view
/// catches up at a bounded speed instead of adding a whole paragraph at once.
#[derive(Default)]
pub struct Reveal {
    shown: usize,
    credit: f32,
}

impl Reveal {
    pub fn complete(text: &str) -> Self {
        Self {
            shown: text.len(),
            credit: 0.0,
        }
    }

    pub fn step(&mut self, text: &str, dt: f32, speed: f32, instant: bool) -> bool {
        if instant {
            self.shown = text.len();
            return false;
        }
        self.shown = self.shown.min(text.len());
        while !text.is_char_boundary(self.shown) {
            self.shown -= 1;
        }
        if self.shown == text.len() {
            self.credit = 0.0;
            return false;
        }
        // Accelerate when the stream outruns the view, while keeping frame time bounded.
        let backlog = text[self.shown..].chars().count();
        self.credit += dt.min(0.1) * speed.max(backlog as f32 * 2.0);
        let count = self.credit.floor() as usize;
        self.credit -= count as f32;
        if count > 0 {
            self.shown = text[self.shown..]
                .char_indices()
                .nth(count)
                .map_or(text.len(), |(offset, _)| self.shown + offset);
        }
        self.shown < text.len()
    }

    pub fn visible<'a>(&self, text: &'a str) -> &'a str {
        &text[..self.shown.min(text.len())]
    }
}

pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revealing_streamed_unicode_preserves_boundaries() {
        let mut reveal = Reveal::default();
        let mut text = "Hej 👋".to_owned();
        for _ in 0..8 {
            reveal.step(&text, 0.02, 30.0, false);
            assert!(text.starts_with(reveal.visible(&text)));
        }
        text.push_str(" 世界 café");
        for _ in 0..100 {
            reveal.step(&text, 0.02, 30.0, false);
        }
        assert_eq!(reveal.visible(&text), text);
    }
    #[test]
    fn reduced_motion_reveals_immediately() {
        let mut reveal = Reveal::default();
        assert!(!reveal.step("hello", 0.0, 0.0, true));
        assert_eq!(reveal.visible("hello"), "hello");
    }
}
