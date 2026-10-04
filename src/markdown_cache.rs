//! Bounded, per-context Markdown parsing cache. Geometry stays owned by egui.
use std::{collections::HashMap, mem::size_of, sync::Arc};

use eframe::egui::{self, Color32, cache::CacheTrait};

use super::{InlineLink, InlineText, inline_text};

const MAX_ENTRIES: usize = 4_096;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct StyleKey {
    size: u32,
    color: Color32,
    strong: bool,
}

struct Entry {
    source: Box<str>,
    style: StyleKey,
    parsed: Arc<InlineText>,
    bytes: usize,
    generation: u32,
}

pub(super) struct InlineCache {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    generation: u32,
    entry_limit: usize,
    byte_limit: usize,
}

impl Default for InlineCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            generation: 0,
            entry_limit: MAX_ENTRIES,
            byte_limit: MAX_BYTES,
        }
    }
}

impl InlineCache {
    pub(super) fn get(
        &mut self,
        text: &str,
        size: f32,
        color: Color32,
        strong: bool,
    ) -> Arc<InlineText> {
        let style = StyleKey {
            size: size.to_bits(),
            color,
            strong,
        };
        // Raw hashes avoid egui's debug-only global Id source registry retaining
        // every changing streaming prefix outside this bounded cache.
        self.get_key(egui::util::hash((text, style)), text, style)
    }

    fn get_key(&mut self, key: u64, text: &str, style: StyleKey) -> Arc<InlineText> {
        if let Some(entry) = self.entries.get_mut(&key)
            && entry.source.as_ref() == text
            && entry.style == style
        {
            entry.generation = self.generation;
            return Arc::clone(&entry.parsed);
        }
        // Verify the complete source/style on hits: even a hash collision must
        // never reuse another line's formatting or link destination.
        if let Some(entry) = self.entries.remove(&key) {
            self.bytes -= entry.bytes;
        }
        let parsed = Arc::new(inline_text(
            text,
            f32::from_bits(style.size),
            style.color,
            style.strong,
        ));
        let bytes = retained_bytes(text, &parsed);
        if self.entries.len() < self.entry_limit
            && self.bytes.saturating_add(bytes) <= self.byte_limit
        {
            self.entries.insert(
                key,
                Entry {
                    source: text.into(),
                    style,
                    parsed: Arc::clone(&parsed),
                    bytes,
                    generation: self.generation,
                },
            );
            self.bytes += bytes;
        }
        // Overflow is an uncached render, not an eviction of still-used lines.
        // This avoids cyclic thrashing on transcripts larger than the cache.
        parsed
    }
}

fn retained_bytes(source: &str, parsed: &InlineText) -> usize {
    // Account for retained source, formatted text, section/link capacities,
    // strings, and entry/Arc metadata. Hash-table and allocator overhead are
    // additionally bounded by MAX_ENTRIES; this is not a peak-process-RAM metric.
    source.len()
        + parsed.job.text.capacity()
        + parsed.job.sections.capacity() * size_of::<egui::text::LayoutSection>()
        + parsed.links.capacity() * size_of::<InlineLink>()
        + parsed
            .links
            .iter()
            .map(|link| link.label.capacity() + link.url.capacity())
            .sum::<usize>()
        + size_of::<InlineText>()
        + size_of::<Entry>()
        + size_of::<u64>()
        + 2 * size_of::<usize>()
}

impl CacheTrait for InlineCache {
    fn update(&mut self) {
        let generation = self.generation;
        self.entries.retain(|_, entry| {
            if entry.generation == generation {
                true
            } else {
                self.bytes -= entry.bytes;
                false
            }
        });
        self.generation = self.generation.wrapping_add(1);
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{MUTED, TEXT};

    #[test]
    fn unchanged_lines_share_parsed_storage_and_stale_lines_are_evicted() {
        let mut cache = InlineCache::default();
        let text = "日本語 **[café `λ`](https://example.com/a(b))**";
        let first = cache.get(text, 14.0, TEXT, false);
        cache.update();
        assert!(Arc::ptr_eq(&first, &cache.get(text, 14.0, TEXT, false)));
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.bytes > 0);
        cache.update();
        cache.update();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
        assert_eq!(first.job.text, "日本語 café λ");
    }

    #[test]
    fn content_font_color_strong_style_and_destinations_are_distinct() {
        let mut cache = InlineCache::default();
        let text = "[docs](https://example.com/first)";
        let first = cache.get(text, 14.0, TEXT, false);
        for (text, size, color, strong) in [
            (text, 16.0, TEXT, false),
            (text, 14.0, MUTED, false),
            (text, 14.0, TEXT, true),
            ("[docs](https://example.com/second)", 14.0, TEXT, false),
            ("changed text", 14.0, TEXT, false),
        ] {
            assert!(!Arc::ptr_eq(&first, &cache.get(text, size, color, strong)));
        }
        assert_eq!(cache.entries.len(), 6);
        assert_eq!(first.links[0].url, "https://example.com/first");
    }

    #[test]
    fn hash_collisions_never_reuse_a_different_source_or_style() {
        let mut cache = InlineCache::default();
        let key = 42; // Force both sources/styles into the same hash bucket.
        let mut style = StyleKey {
            size: 14.0_f32.to_bits(),
            color: TEXT,
            strong: false,
        };
        let first = cache.get_key(key, "[docs](https://example.com/first)", style);
        let second = cache.get_key(key, "[docs](https://example.com/second)", style);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.links[0].url, "https://example.com/second");
        style.strong = true;
        assert!(!Arc::ptr_eq(
            &second,
            &cache.get_key(key, "[docs](https://example.com/second)", style)
        ));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(
            cache.bytes,
            cache
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn entry_and_byte_caps_bypass_without_churning_live_entries() {
        let mut cache = InlineCache {
            entry_limit: 2,
            ..Default::default()
        };
        let first = cache.get("first", 14.0, TEXT, false);
        let second = cache.get("second", 14.0, TEXT, false);
        let third = cache.get("third", 14.0, TEXT, false);
        assert_eq!(cache.entries.len(), 2);
        assert!(!Arc::ptr_eq(&third, &cache.get("third", 14.0, TEXT, false)));
        cache.update();
        assert!(Arc::ptr_eq(&first, &cache.get("first", 14.0, TEXT, false)));
        assert!(Arc::ptr_eq(
            &second,
            &cache.get("second", 14.0, TEXT, false)
        ));
        cache.update();
        cache.update();
        assert_eq!(cache.bytes, 0);

        let mut cache = InlineCache {
            byte_limit: 1,
            ..Default::default()
        };
        let bypassed = cache.get("**still rendered**", 14.0, TEXT, false);
        assert_eq!(bypassed.job.text, "still rendered");
        assert_eq!(Arc::strong_count(&bypassed), 1);
        let owned = Arc::try_unwrap(bypassed)
            .ok()
            .expect("uncached lines retain the owned rendering path");
        assert_eq!(owned.job.text, "still rendered");
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn byte_caps_allow_exact_fit_but_bypass_additional_lines() {
        let text = "[first](https://example.com/first)";
        let bytes = retained_bytes(text, &inline_text(text, 14.0, TEXT, false));
        let mut cache = InlineCache {
            byte_limit: bytes,
            ..Default::default()
        };
        let first = cache.get(text, 14.0, TEXT, false);
        assert_eq!(cache.bytes, bytes);
        let extra = cache.get("another line", 14.0, TEXT, false);
        assert_eq!(extra.job.text, "another line");
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, bytes);
        assert!(Arc::ptr_eq(&first, &cache.get(text, 14.0, TEXT, false)));
        cache.update();
        cache.update();
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn streamed_unicode_prefixes_and_unsafe_destinations_never_reuse_stale_links() {
        let mut cache = InlineCache::default();
        let text = "Before [次へ](https://example.com/docs)";
        for (end, _) in text.char_indices() {
            let prefix = &text[..end];
            let parsed = cache.get(prefix, 14.0, TEXT, false);
            assert_eq!(parsed.job.text, prefix);
            assert!(parsed.links.is_empty());
            cache.update();
            assert_eq!(cache.entries.len(), 1);
        }
        let complete = cache.get(text, 14.0, TEXT, false);
        assert_eq!(complete.job.text, "Before 次へ");
        assert_eq!(complete.links[0].url, "https://example.com/docs");
        cache.update();
        let unsafe_text = "Before [次へ](javascript:alert(1))";
        let changed = cache.get(unsafe_text, 14.0, TEXT, false);
        assert_eq!(changed.job.text, unsafe_text);
        assert!(changed.links.is_empty());
        cache.update();
        assert_eq!(cache.entries.len(), 1);
        assert!(!Arc::ptr_eq(&complete, &changed));
    }
}
