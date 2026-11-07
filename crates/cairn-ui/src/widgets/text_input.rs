//! Single-line text input: caret movement, deletion, selection, clipboard and IME preedit.

use cairn_ui_layout::MeasureInput;

use crate::anim::Transition;
use crate::element::element;
use crate::text_edit::EditState;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::drive;
use crate::widgets::icon::{Icon, paint_icon};
use crate::{
    CursorIcon, FontFamily, ImeEvent, Insets, Key, KeyEvent, NamedKey, Painter, Point, PointerButton, Rect, Shadow, Size, Style, TextLayout, TextStyle,
};

/// Content width in logical pixels when no width is given.
const DEFAULT_WIDTH: f32 = 200.0;

const MASK: char = '•';

/// Text being composed by the IME, not yet committed. It is shown at the caret and is not
/// part of the field value.
struct Preedit {
    text: String,
    /// Byte range in the preedit text, given by the IME. Equal ends mean a caret position;
    /// otherwise it is the clause being converted.
    cursor: Option<(usize, usize)>,
}

type TextHandler<S> = Box<dyn Fn(&mut S, String)>;

/// Single-line text input. Create it with [`text_input`].
///
/// The app owns the value. Pass in the current value and receive edits through
/// [`TextInput::on_change`]. With `on_change` set the field is controlled: on each rebuild, a
/// passed value that differs from the text wins, for example when the app clears it. Without
/// `on_change`, the text is replaced only when the passed value itself changes.
pub struct TextInput<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    value: String,
    placeholder: String,
    on_change: Option<TextHandler<S>>,
    on_submit: Option<TextHandler<S>>,
    edit: EditState,
    preedit: Option<Preedit>,
    /// Layout of the shown text: the value with the preedit text inserted at the caret.
    display: Option<TextLayout>,
    hint: Option<TextLayout>,
    scroll_x: f32,
    content_width: f32,
    caret_on: bool,
    dragging: bool,
    hover: Transition,
    focus: Transition,
    icon: Option<Icon>,
    /// Password field: text is drawn as dots and cannot be copied or cut.
    /// The IME and screen readers do not get the text.
    secret: bool,
}

pub fn text_input<S: 'static>(value: impl Into<String>) -> TextInput<S> {
    let value = value.into();
    TextInput {
        style: Style { padding: Insets::AUTO, ..Style::default() },
        key: None,
        edit: EditState::new(value.clone()),
        value,
        placeholder: String::new(),
        on_change: None,
        on_submit: None,
        preedit: None,
        display: None,
        hint: None,
        scroll_x: 0.0,
        content_width: DEFAULT_WIDTH,
        caret_on: true,
        dragging: false,
        hover: Transition::default(),
        focus: Transition::default(),
        icon: None,
        secret: false,
    }
}

element!(TextInput<S>);

/// Drops control characters; line breaks become spaces, as when pasting multi-line text.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

impl<S: 'static> TextInput<S> {
    /// Icon shown before the text, such as a magnifier in a search field.
    /// It is drawn in the left padding and styled by the theme's `input.icon*` fields.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn on_change(mut self, f: impl Fn(&mut S, String) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// Called when Enter is pressed.
    pub fn on_submit(mut self, f: impl Fn(&mut S, String) + 'static) -> Self {
        self.on_submit = Some(Box::new(f));
        self
    }

    fn text_style(theme: &Theme) -> TextStyle {
        let body = theme.typography.body;
        TextStyle { font_size: body.size, line_height: theme.typography.line_height, weight: body.weight, family: FontFamily::SansSerif }
    }

    fn display_text(&self) -> String {
        let text = self.edit.text();
        if self.secret {
            return MASK.to_string().repeat(text.chars().count());
        }
        match &self.preedit {
            None => text.to_string(),
            Some(pe) => {
                let c = self.edit.cursor();
                format!("{}{}{}", &text[..c], pe.text, &text[c..])
            }
        }
    }

    /// Maps a byte offset in the value to one in the displayed text. A secret field shows one
    /// dot per character.
    fn to_display(&self, i: usize) -> usize {
        if !self.secret {
            return i;
        }
        self.edit.text()[..i].chars().count() * MASK.len_utf8()
    }

    fn from_display(&self, j: usize) -> usize {
        if !self.secret {
            return j;
        }
        let text = self.edit.text();
        text.char_indices().nth(j / MASK.len_utf8()).map_or(text.len(), |(b, _)| b)
    }

    /// Caret position in the displayed text. Returns `None` while the IME is converting a
    /// clause, when no caret is shown.
    fn caret_index(&self) -> Option<usize> {
        let c = self.to_display(self.edit.cursor());
        match &self.preedit {
            None => Some(c),
            Some(pe) => match pe.cursor {
                Some((a, b)) if a == b => Some(c + b.min(pe.text.len())),
                Some(_) => None,
                None => Some(c + pe.text.len()),
            },
        }
    }

    fn sync_layouts(&mut self, cx: &mut LayoutCx) {
        let style = Self::text_style(cx.theme());
        let text = self.display_text();
        match &mut self.display {
            Some(d) => {
                d.set_style(cx.text(), &style);
                d.set_text(cx.text(), &text);
            }
            None => self.display = Some(TextLayout::new(cx.text(), &text, &style)),
        }
        match &mut self.hint {
            Some(h) => {
                h.set_style(cx.text(), &style);
                h.set_text(cx.text(), &self.placeholder);
            }
            None => self.hint = Some(TextLayout::new(cx.text(), &self.placeholder, &style)),
        }
    }

    fn line_height(&self) -> f32 {
        self.display.as_ref().map(|d| d.line_height()).unwrap_or(20.0)
    }

    fn text_origin(&self, content: Rect) -> Point {
        Point::new(content.x - self.scroll_x, content.y + (content.height - self.line_height()) / 2.0)
    }

    /// Caret rect in window coordinates. Its width is one physical pixel, or two at 150% scale
    /// and up.
    fn caret_rect(&self, content: Rect, scale: f32) -> Option<Rect> {
        let display = self.display.as_ref()?;
        let pos = display.cursor_position(self.caret_index()?);
        let origin = self.text_origin(content);
        let w = scale.round().max(1.0) / scale;
        Some(Rect::new(origin.x + pos.x, origin.y + pos.y, w, display.line_height()))
    }

    fn scroll_to_caret(&mut self) {
        let Some(display) = &self.display else { return };
        const CARET_ROOM: f32 = 2.0;
        let w = self.content_width;
        let index = self.caret_index().unwrap_or(self.edit.cursor());
        let x = display.cursor_position(index).x;
        let max_scroll = (display.size().width + CARET_ROOM - w).max(0.0);
        let mut s = self.scroll_x;
        if x - s > w - CARET_ROOM {
            s = x - w + CARET_ROOM;
        }
        if x - s < 0.0 {
            s = x;
        }
        self.scroll_x = s.clamp(0.0, max_scroll);
    }

    fn hit_index(&self, content: Rect, pos: Point) -> usize {
        match &self.display {
            Some(d) => self.from_display(d.hit(Point::new(pos.x - content.x + self.scroll_x, d.line_height() / 2.0))),
            None => 0,
        }
    }

    fn reset_blink(&mut self, cx: &mut EventCx<S>) {
        self.caret_on = true;
        if let Some(interval) = cx.caret_blink_interval() {
            cx.request_timer(cx.now() + interval);
        }
    }

    fn moved(&mut self, cx: &mut EventCx<S>) {
        self.scroll_to_caret();
        self.reset_blink(cx);
        cx.request_paint();
    }

    fn relayout(&mut self, cx: &mut EventCx<S>) {
        self.sync_layouts(&mut cx.layout_cx());
        self.moved(cx);
    }

    fn edited(&mut self, cx: &mut EventCx<S>) {
        self.relayout(cx);
        if let Some(f) = &self.on_change {
            f(cx.app(), self.edit.text().to_string());
        }
    }

    fn key(&mut self, cx: &mut EventCx<S>, k: &KeyEvent) {
        let m = cx.modifiers();
        let (word, shift) = (m.word_jump(), m.shift);
        let mut handled = true;
        let mut edited = false;
        let mut moved = false;
        match &k.key {
            // The IME is handling this key. Wait for its Preedit or Commit event.
            Key::Named(NamedKey::Process) => return,
            Key::Named(NamedKey::ArrowLeft) => {
                self.edit.move_left(word, shift);
                moved = true;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.edit.move_right(word, shift);
                moved = true;
            }
            Key::Named(NamedKey::Home) => {
                self.edit.home(shift);
                moved = true;
            }
            Key::Named(NamedKey::End) => {
                self.edit.end(shift);
                moved = true;
            }
            Key::Named(NamedKey::Backspace) => edited = self.edit.backspace(word),
            Key::Named(NamedKey::Delete) => edited = self.edit.delete_forward(word),
            Key::Named(NamedKey::Enter) => {
                if let Some(f) = &self.on_submit {
                    f(cx.app(), self.edit.text().to_string());
                }
            }
            Key::Character(c) if m.shortcut() && !m.alt => match c.to_lowercase().as_str() {
                "a" => {
                    self.edit.select_all();
                    moved = true;
                }
                "c" | "x" if self.secret => {}
                "c" => {
                    if self.edit.has_selection() {
                        cx.clipboard_set(self.edit.selected_text());
                    }
                }
                "x" => {
                    if self.edit.has_selection() {
                        cx.clipboard_set(self.edit.selected_text());
                        edited = self.edit.delete_selection();
                    }
                }
                "v" => {
                    let text = cx.clipboard_get().map(|t| sanitize(&t)).unwrap_or_default();
                    if !text.is_empty() {
                        self.edit.insert(&text);
                        edited = true;
                    }
                }
                _ => handled = false,
            },
            _ => {
                // Ctrl+letter is not text input.
                // Characters typed with AltGr (Ctrl+Alt on Windows) are text.
                let text = k.text.as_deref().filter(|_| !m.ctrl || m.alt).map(sanitize).unwrap_or_default();
                if text.is_empty() {
                    handled = false;
                } else {
                    self.edit.insert(&text);
                    edited = true;
                }
            }
        }
        if handled {
            cx.set_handled();
        }
        if edited {
            self.edited(cx);
        } else if moved {
            self.moved(cx);
        }
    }

    fn ime(&mut self, cx: &mut EventCx<S>, ev: &ImeEvent) {
        cx.set_handled();
        match ev {
            ImeEvent::Enabled => {}
            ImeEvent::Preedit { text, cursor } if !text.is_empty() => {
                // Composing over a selection deletes the selection first, as most editors do.
                // The candidate then replaces the selected text.
                let deleted = self.preedit.is_none() && self.edit.delete_selection();
                self.preedit = Some(Preedit { text: text.clone(), cursor: *cursor });
                if deleted { self.edited(cx) } else { self.relayout(cx) }
            }
            ImeEvent::Preedit { .. } | ImeEvent::Disabled => {
                if self.preedit.take().is_some() {
                    self.relayout(cx);
                }
            }
            ImeEvent::Commit(text) => {
                self.preedit = None;
                let text = sanitize(text);
                if text.is_empty() {
                    self.relayout(cx);
                } else {
                    self.edit.insert(&text);
                    self.edited(cx);
                }
            }
        }
    }
}

impl<S: 'static> Widget<S> for TextInput<S> {
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change {
        if new.icon != self.icon {
            self.icon = new.icon;
            // Padding is recomputed from the new icon in `resolve_style`.
            // A style change re-lays out, so no change flag is needed.
        }
        let mut change = Change::NONE;
        let controlled = new.on_change.is_some();
        let external = new.value != self.value;
        if (controlled && new.value != self.edit.text()) || (!controlled && external) {
            self.edit.replace_text(new.value.clone());
            self.preedit = None;
            change = Change::PAINT;
        }
        if new.placeholder != self.placeholder || new.secret != self.secret {
            change = Change::PAINT;
        }
        self.secret = new.secret;
        self.value = new.value;
        self.placeholder = new.placeholder;
        self.on_change = new.on_change;
        self.on_submit = new.on_submit;
        if !change.is_none() {
            self.sync_layouts(cx);
            self.scroll_to_caret();
        }
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = theme.input.padding;
        }
        // The icon is drawn in the left padding, so the padding grows by the icon size and gap.
        if self.icon.is_some() {
            style.padding.left += theme.input.icon_size + theme.input.icon_gap;
        }
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        self.sync_layouts(cx);
        Size::new(DEFAULT_WIDTH, self.line_height())
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = theme.input.focus_glow.extent();
        bounds.outset(e[0], e[1], e[2], e[3])
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        self.content_width = content.width;
        self.sync_layouts(cx);
        self.scroll_to_caret();
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let motion = &cx.theme().motion;
        let (focused, hovered) = (cx.is_focused(), cx.is_hovered());
        drive(&mut self.hover, cx, hovered, motion.hover, motion.easing);
        drive(&mut self.focus, cx, focused, motion.focus, motion.easing);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.input;
        let (b, c) = (cx.bounds(), cx.content());
        let focused = cx.is_focused();
        let now = cx.now();
        let (hover, focus) = (self.hover.value(now), self.focus.value(now));
        let radius = theme.radius.control;

        let glow = Shadow { color: look.focus_glow.color.fade(focus), ..look.focus_glow };
        p.draw_shadow(b, radius, &glow);
        p.fill_rounded_rect(b, radius, look.fill.mix(look.fill_hover, hover).mix(look.fill_focus, focus));
        let border = look.border.mix(look.border_hover, hover).mix(look.border_focus, focus);
        let width = if focused { look.border_focus_width } else { look.border_width };
        p.stroke_rect(b, radius, width, border);
        if let Some(icon) = self.icon {
            let s = look.icon_size;
            let r = Rect::new(c.x - look.icon_gap - s, b.y + (b.height - s) / 2.0, s, s);
            paint_icon(p, icon, r, look.icon);
        }

        let (Some(display), Some(hint)) = (&self.display, &self.hint) else {
            return;
        };
        let origin = self.text_origin(c);
        let px = p.scale().round().max(1.0) / p.scale();
        p.push_clip(c);
        if display.text().is_empty() {
            p.draw_text(hint, Point::new(c.x, origin.y), look.placeholder);
        } else {
            if focused && self.preedit.is_none() && self.edit.has_selection() {
                let sel = self.edit.selection();
                for r in display.selection_rects(self.to_display(sel.start)..self.to_display(sel.end)) {
                    p.fill_rect(r.translate(origin), look.selection);
                }
            }
            p.draw_text(display, origin, look.text);
            if let Some(pe) = &self.preedit {
                let start = self.edit.cursor();
                let y = origin.y + display.first_baseline() + 2.0 * px.max(1.0);
                for r in display.selection_rects(start..start + pe.text.len()) {
                    p.fill_rect(Rect::new(origin.x + r.x, y, r.width, px), look.preedit);
                }
                if let Some((a, z)) = pe.cursor.filter(|(a, z)| a < z) {
                    for r in display.selection_rects(start + a..start + z.min(pe.text.len())) {
                        p.fill_rect(Rect::new(origin.x + r.x, y, r.width, 2.0 * px), look.preedit);
                    }
                }
            }
        }
        if focused
            && cx.window_focused()
            && self.caret_on
            && let Some(caret) = self.caret_rect(c, p.scale())
        {
            p.fill_rect(caret, look.caret);
        }
        p.pop_clip();
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerEnter | Event::PointerLeave => cx.request_paint(),
            Event::FocusGained => {
                self.reset_blink(cx);
                cx.request_paint();
            }
            Event::FocusLost => {
                self.dragging = false;
                if self.preedit.take().is_some() {
                    self.relayout(cx);
                }
                cx.request_paint();
            }
            Event::WindowFocusChanged(focused) => {
                if *focused {
                    self.reset_blink(cx);
                }
                cx.request_paint();
            }
            Event::Timer => {
                // Once focus is lost the timer is not re-armed, so blinking stops by itself.
                if cx.is_focused()
                    && cx.window_focused()
                    && let Some(interval) = cx.caret_blink_interval()
                {
                    self.caret_on = !self.caret_on;
                    if let Some(r) = self.caret_rect(cx.content(), cx.scale()) {
                        cx.request_paint_rect(r.inflate(1.0, 1.0));
                    }
                    cx.request_timer(cx.now() + interval);
                }
            }
            Event::PointerDown { pos, button: PointerButton::Left, count } => {
                cx.set_handled();
                // While composing, leave clicks to the IME, which commits or cancels on its own.
                if self.preedit.is_some() {
                    return;
                }
                let index = self.hit_index(cx.content(), *pos);
                match count {
                    1 => {
                        self.edit.set_cursor(index, cx.modifiers().shift);
                        self.dragging = true;
                    }
                    2 => self.edit.select_word_at(index),
                    _ => self.edit.select_all(),
                }
                self.moved(cx);
            }
            Event::PointerMove { pos } if self.dragging => {
                let index = self.hit_index(cx.content(), *pos);
                self.edit.set_cursor(index, true);
                self.moved(cx);
            }
            Event::PointerUp { button: PointerButton::Left, .. } => {
                self.dragging = false;
                cx.set_handled();
            }
            Event::Key(k) if k.pressed => self.key(cx, k),
            Event::Ime(ime) => self.ime(cx, ime),
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn wants_ime(&self) -> bool {
        !self.secret
    }

    fn ime_area(&self, bounds: Rect, content: Rect) -> Option<Rect> {
        let display = self.display.as_ref()?;
        // While composing, anchor at the preedit start so the candidate box does not jump with
        // the typed pinyin. Otherwise anchor at the caret.
        let index = if self.preedit.is_some() { self.edit.cursor() } else { self.caret_index()? };
        let x = (self.text_origin(content).x + display.cursor_position(index).x).clamp(content.x, content.right());
        // Vertically it spans the whole field, so the candidate box opens below, not over it.
        Some(Rect::new(x, bounds.y, 1.0, bounds.height))
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Text
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::TextInput,
            label: (!self.placeholder.is_empty()).then(|| self.placeholder.clone()),
            value: (!self.secret).then(|| self.edit.text().to_string()),
            disabled: false,
        }
    }
}
