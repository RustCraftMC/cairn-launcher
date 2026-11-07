//! Text layout, font management via cosmic-text, and cursor/selection queries.
//! Layout is performed in logical pixels, independent of the scale factor.

use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Cursor, Family, FontSystem, LineIter, Metrics, Shaping, SwashCache, Weight, Wrap, fontdb};

use crate::geom::{Point, Rect, Size};

/// Font configuration specifying directories, preferred families, and locale.
#[derive(Clone, Debug, Default)]
pub struct FontSetup {
    /// Directories scanned recursively for font files (`ttf`, `otf`, `ttc`, `otc`).
    pub dirs: Vec<PathBuf>,
    /// Candidate font family names in priority order.
    pub sans_serif: Vec<String>,
    pub serif: Vec<String>,
    pub monospace: Vec<String>,
    /// BCP 47 language tag (e.g. `zh-CN`) controlling CJK font fallback.
    pub locale: String,
}

/// Shared font database and rasterized glyph cache.
pub struct TextSystem {
    pub(crate) fonts: FontSystem,
    pub(crate) glyphs: SwashCache,
    load_time: Duration,
    sans_serif: Option<String>,
    checked_glyphs: usize,
}

impl TextSystem {
    pub fn new(setup: &FontSetup) -> Self {
        let start = Instant::now();
        let mut db = fontdb::Database::new();
        for dir in &setup.dirs {
            db.load_fonts_dir(dir);
        }
        let pick = |db: &fontdb::Database, names: &[String]| {
            names.iter().find(|name| db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))).cloned()
        };
        let sans_serif = pick(&db, &setup.sans_serif);
        if let Some(name) = &sans_serif {
            db.set_sans_serif_family(name.clone());
        }
        if let Some(name) = pick(&db, &setup.serif) {
            db.set_serif_family(name);
        }
        if let Some(name) = pick(&db, &setup.monospace) {
            db.set_monospace_family(name);
        }
        let locale = if setup.locale.is_empty() { "en-US".to_string() } else { setup.locale.clone() };
        let fonts = FontSystem::new_with_locale_and_db(locale, db);
        TextSystem { fonts, glyphs: SwashCache::new(), load_time: start.elapsed(), sans_serif, checked_glyphs: 0 }
    }

    pub fn face_count(&self) -> usize {
        self.fonts.db().len()
    }

    /// Time spent scanning directories and parsing font headers.
    pub fn load_time(&self) -> Duration {
        self.load_time
    }

    /// Selected sans-serif default font family, or `None` if no candidate was found.
    pub fn sans_serif_family(&self) -> Option<&str> {
        self.sans_serif.as_deref()
    }

    /// Clears the rasterized glyph cache to reclaim memory after scale changes.
    pub fn clear_glyph_cache(&mut self) {
        self.glyphs.image_cache.clear();
        self.glyphs.outline_command_cache.clear();
        self.checked_glyphs = 0;
    }

    pub fn trim_glyph_cache(&mut self) {
        let count = self.glyphs.image_cache.len();
        if count == self.checked_glyphs {
            return;
        }
        let bytes: usize = self.glyphs.image_cache.values().flatten().map(|image| image.data.len()).sum();
        if count > 8192 || bytes > 16 << 20 {
            self.clear_glyph_cache();
        } else {
            self.checked_glyphs = count;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

/// Text styling parameters in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    /// Line height multiple relative to font size.
    pub line_height: f32,
    /// Font weight (400 for regular, 700 for bold).
    pub weight: u16,
    pub family: FontFamily,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle { font_size: 14.0, line_height: 1.4, weight: 400, family: FontFamily::SansSerif }
    }
}

impl TextStyle {
    fn metrics(&self) -> Metrics {
        Metrics::new(self.font_size, (self.font_size * self.line_height).round().max(1.0))
    }

    fn attrs(&self) -> Attrs<'_> {
        let family = match &self.family {
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Serif => Family::Serif,
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Named(name) => Family::Name(name),
        };
        // Normalize weight to 400 or 700 per CSS specs to avoid mismatched Japanese fallback fonts on Windows.
        let weight = if self.weight >= 600 { 700 } else { 400 };
        Attrs::new()
            .family(family)
            .weight(Weight(weight))
            // Disable bytecode hinting to prevent stroke collapse or distortion at small font sizes.
            .cache_key_flags(CacheKeyFlags::DISABLE_HINTING)
    }
}

/// Laid out text buffer with reusable internal storage.
pub struct TextLayout {
    pub(crate) buffer: Buffer,
    text: String,
    style: TextStyle,
    wrap_width: Option<f32>,
    /// Starting byte index of each logical line in the source text.
    line_starts: Vec<usize>,
    size: Size,
}

impl TextLayout {
    pub fn new(ts: &mut TextSystem, text: &str, style: &TextStyle) -> Self {
        let mut buffer = Buffer::new(&mut ts.fonts, style.metrics());
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        let mut layout = TextLayout { buffer, text: String::new(), style: style.clone(), wrap_width: None, line_starts: vec![0], size: Size::ZERO };
        layout.reshape(ts, text);
        layout
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> &TextStyle {
        &self.style
    }

    /// Updates the text content. Returns true if the text actually changed.
    pub fn set_text(&mut self, ts: &mut TextSystem, text: &str) -> bool {
        if text == self.text {
            return false;
        }
        self.reshape(ts, text);
        true
    }

    /// Updates the text style. Returns true if the style actually changed.
    pub fn set_style(&mut self, ts: &mut TextSystem, style: &TextStyle) -> bool {
        if *style == self.style {
            return false;
        }
        self.style = style.clone();
        self.buffer.set_metrics(self.style.metrics());
        let text = std::mem::take(&mut self.text);
        self.reshape(ts, &text);
        true
    }

    /// Sets the wrapping width, or `None` to disable line wrapping.
    pub fn set_wrap_width(&mut self, ts: &mut TextSystem, width: Option<f32>) {
        let width = width.map(|w| w.max(0.0));
        if width == self.wrap_width {
            return;
        }
        self.wrap_width = width;
        self.buffer.set_wrap(if width.is_some() { Wrap::WordOrGlyph } else { Wrap::None });
        self.buffer.set_size(width, None);
        self.relayout(ts);
    }

    fn reshape(&mut self, ts: &mut TextSystem, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.buffer.set_text(text, &self.style.attrs(), Shaping::Advanced, None);
        self.line_starts.clear();
        let mut last_had_ending = true;
        for (range, ending) in LineIter::new(text) {
            self.line_starts.push(range.start);
            last_had_ending = ending != cosmic_text::LineEnding::None;
        }
        if last_had_ending {
            // Append an empty trailing line if the text ends with a newline or is empty.
            self.line_starts.push(text.len());
        }
        self.relayout(ts);
    }

    fn relayout(&mut self, ts: &mut TextSystem) {
        self.buffer.shape_until_scroll(&mut ts.fonts, false);
        let mut w: f32 = 0.0;
        let mut h: f32 = 0.0;
        for run in self.buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        self.size = Size::new(w, h.max(self.line_height()));
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn line_height(&self) -> f32 {
        self.buffer.metrics().line_height
    }

    /// Distance from the top edge to the baseline of the first line.
    pub fn first_baseline(&self) -> f32 {
        self.buffer.layout_runs().next().map(|r| r.line_y).unwrap_or(self.line_height() * 0.8)
    }

    fn index_to_cursor(&self, index: usize) -> Cursor {
        let index = index.min(self.text.len());
        let line = self.line_starts.partition_point(|&s| s <= index).saturating_sub(1);
        Cursor::new(line, index - self.line_starts[line])
    }

    fn cursor_to_index(&self, c: Cursor) -> usize {
        let start = self.line_starts.get(c.line).copied().unwrap_or(self.text.len());
        (start + c.index).min(self.text.len())
    }

    /// Finds the byte index on a grapheme cluster boundary nearest to the given point.
    pub fn hit(&self, p: Point) -> usize {
        match self.buffer.hit(p.x, p.y) {
            Some(c) => self.cursor_to_index(c),
            None if p.y < 0.0 => 0,
            None => self.text.len(),
        }
    }

    /// Returns the cursor top-left point at `index`, with height `Self::line_height`.
    pub fn cursor_position(&self, index: usize) -> Point {
        let cursor = self.index_to_cursor(index);
        let mut fallback = None;
        for run in self.buffer.layout_runs().filter(|r| r.line_i == cursor.line) {
            if let Some(x) = run.cursor_position(&cursor) {
                return Point::new(x, run.line_top);
            }
            fallback = Some(Point::new(run.line_w, run.line_top));
        }
        fallback.unwrap_or(Point::ZERO)
    }

    /// Selection rectangles for the given byte range across wrapped lines or bidi text.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut rects = Vec::new();
        if range.start >= range.end {
            return rects;
        }
        let (start, end) = (self.index_to_cursor(range.start), self.index_to_cursor(range.end));
        for run in self.buffer.layout_runs() {
            if run.line_i < start.line || run.line_i > end.line {
                continue;
            }
            for (x, w) in run.highlight(start, end) {
                rects.push(Rect::new(x, run.line_top, w, run.line_height));
            }
        }
        rects
    }
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayout").field("text", &self.text).field("size", &self.size).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_cache_limits_pixels_and_missing_glyph_entries() {
        let mut text = TextSystem::new(&FontSetup::default());
        let key = |glyph| cosmic_text::CacheKey::new(fontdb::ID::dummy(), glyph, 14.0, (0.0, 0.0), fontdb::Weight::NORMAL, CacheKeyFlags::empty()).0;
        let mut image = cosmic_text::SwashImage::new();
        image.data = vec![0; (16 << 20) + 1];
        text.glyphs.image_cache.insert(key(0), Some(image));
        text.trim_glyph_cache();
        assert!(text.glyphs.image_cache.is_empty());
        for glyph in 0..8192 {
            text.glyphs.image_cache.insert(key(glyph), None);
        }
        text.trim_glyph_cache();
        assert_eq!(text.glyphs.image_cache.len(), 8192);
        text.glyphs.image_cache.insert(key(8192), None);
        text.trim_glyph_cache();
        assert!(text.glyphs.image_cache.is_empty());
    }

    #[test]
    fn attrs_have_disable_hinting_flag_and_quantized_weight() {
        let regular = TextStyle { weight: 500, ..TextStyle::default() };
        assert_eq!(regular.attrs().weight, Weight(400));
        assert!(regular.attrs().cache_key_flags.contains(CacheKeyFlags::DISABLE_HINTING));

        let bold = TextStyle { weight: 600, ..TextStyle::default() };
        assert_eq!(bold.attrs().weight, Weight(700));
        assert!(bold.attrs().cache_key_flags.contains(CacheKeyFlags::DISABLE_HINTING));
    }
}
