//! Drives the UI without a window for tests and screenshots, using the real runtime.
//! Platform services (IME, clipboard) are recording fakes that tests can read back.
//! Fonts are the system fonts, so results depend on what is installed.

use std::time::{Duration, Instant};

use cairn_ui_platform::{self as platform, ImeEvent, Key, KeyEvent, Modifiers, PixelRect, PointerButton, ScrollDelta};

use crate::app::{ViewFn, erase_view, system_fonts};
use crate::host::{FRAME_INTERVAL, RecordingHost};
use crate::theme::Theme;
use crate::tree::WidgetId;
use crate::ui::{Clock, Ui};
use crate::widget::{Role, Semantics};
use crate::{DeviceRect, Point, Rect, TextSystem};

pub fn system_font_setup() -> crate::FontSetup {
    system_fonts()
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    pub id: WidgetId,
    pub semantics: Semantics,
    /// Bounding box in window coordinates, in logical pixels.
    pub bounds: Rect,
    pub focused: bool,
}

/// Time is virtual and starts at creation. Only `Headless::advance` and
/// `Headless::fire_timers` move it, so animation states can be reproduced exactly.
/// Animation frames are 60 Hz.
pub struct Headless<S: 'static> {
    ui: Ui<S>,
    host: RecordingHost,
}

impl<S: 'static> Headless<S> {
    /// `width` and `height` are logical pixels. `scale` is the factor, 1.25 for 125%.
    pub fn new<F>(state: S, view: F, width: f32, height: f32, scale: f32) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        Self::with_text_system(state, view, width, height, scale, TextSystem::new(&system_fonts()))
    }

    /// Reuses a loaded font library. Loading system fonts is slow,
    /// so tests can share one.
    pub fn with_text_system<F>(state: S, view: F, width: f32, height: f32, scale: f32, text: TextSystem) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        Self::with_theme(state, view, width, height, scale, Theme::default(), text)
    }

    pub fn with_theme<F>(state: S, view: F, width: f32, height: f32, scale: f32, theme: Theme, text: TextSystem) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        let phys = ((width * scale).round() as u32, (height * scale).round() as u32);
        let clock = Clock::Manual(Instant::now());
        let mut ui = Ui::new(state, erase_view(view), theme, text, scale, phys, clock);
        let mut host = RecordingHost::default();
        ui.after(&mut host);
        Headless { ui, host }
    }

    pub fn into_text_system(self) -> TextSystem {
        self.ui.text
    }

    pub fn state(&self) -> &S {
        &self.ui.state
    }

    pub fn theme_from(&mut self, f: impl Fn(&mut S) -> Option<Theme> + 'static) {
        self.ui.set_theme_source(Some(Box::new(f)), false);
        self.ui.after(&mut self.host);
    }

    /// Changes the state the way a background thread does through `crate::Proxy::update`.
    pub fn update(&mut self, f: impl FnOnce(&mut S) + Send + 'static) {
        let f: crate::ui::Update<S> = Box::new(f);
        self.ui.user_event(&mut self.host, Box::new(f));
    }

    /// Redraws the dirty regions. Returns the updated regions, in physical pixels.
    pub fn render(&mut self) -> Vec<DeviceRect> {
        self.host.redraw_requested = false;
        self.ui.render(&mut self.host)
    }

    pub fn now(&self) -> Instant {
        self.ui.clock.now()
    }

    /// Moves the virtual clock forward by `d`, firing due timers and animation frames.
    /// Does not redraw; call `render` to paint the changes.
    pub fn advance(&mut self, d: Duration) {
        let now = self.now() + d;
        self.fire_timers(now);
    }

    pub fn animating(&self) -> bool {
        self.ui.animating()
    }

    /// Steps and redraws frame by frame until no animation is left (at most `max_frames`).
    /// Returns the dirty regions redrawn in each frame.
    pub fn run_animations(&mut self, max_frames: usize) -> Vec<Vec<DeviceRect>> {
        let mut frames = Vec::new();
        while frames.len() < max_frames {
            let Some(at) = self.ui.next_frame() else { break };
            self.fire_timers(at.max(self.now()));
            frames.push(self.render());
        }
        frames
    }

    /// Canvas pixels as RGBA8: `width() * height()` pixels, 4 bytes each.
    pub fn pixels(&self) -> &[u8] {
        self.ui.canvas.pixels()
    }

    /// Glass coverage in compositing mode: one byte per pixel.
    /// See [`Headless::set_compositing`].
    pub fn glass(&self) -> &[u8] {
        self.ui.canvas.glass()
    }

    pub fn width(&self) -> u32 {
        self.ui.canvas.width()
    }

    pub fn height(&self) -> u32 {
        self.ui.canvas.height()
    }

    pub fn png(&self) -> Vec<u8> {
        encode_png(self.pixels(), self.width(), self.height())
    }

    /// Whether a redraw was requested since the last `render`.
    pub fn redraw_requested(&self) -> bool {
        self.host.redraw_requested
    }

    /// All visible nodes in Tab order: depth first, parents before children.
    pub fn nodes(&self) -> Vec<NodeInfo> {
        let tree = &self.ui.tree;
        tree.visible_in_order()
            .into_iter()
            .map(|id| {
                let n = &tree.nodes[id];
                NodeInfo { id, semantics: n.widget.semantics(), bounds: n.bounds, focused: self.ui.focused == Some(id) }
            })
            .collect()
    }

    /// The first node whose name or value equals `text`.
    pub fn find(&self, text: &str) -> Option<NodeInfo> {
        self.nodes().into_iter().find(|n| n.semantics.label.as_deref() == Some(text) || n.semantics.value.as_deref() == Some(text))
    }

    pub fn find_role(&self, role: Role) -> Vec<NodeInfo> {
        self.nodes().into_iter().filter(|n| n.semantics.role == role).collect()
    }

    fn send(&mut self, e: platform::Event) {
        self.ui.handle(&mut self.host, e);
    }

    pub fn pointer_move(&mut self, p: Point) {
        let s = self.ui.scale() as f64;
        self.send(platform::Event::PointerMoved { x: p.x as f64 * s, y: p.y as f64 * s });
    }

    pub fn pointer_leave(&mut self) {
        self.send(platform::Event::PointerLeft);
    }

    pub fn pointer_button(&mut self, button: PointerButton, pressed: bool) {
        self.send(platform::Event::PointerButton { button, pressed });
    }

    pub fn exit_requested(&self) -> bool {
        self.host.exit_requested
    }

    /// Returns (is maximized, was minimize requested).
    /// The fake host flips the maximized flag when the maximize button is clicked.
    pub fn window_state(&self) -> (bool, bool) {
        (self.host.maximized, self.host.minimized)
    }

    pub fn click(&mut self, p: Point) {
        self.pointer_move(p);
        self.pointer_button(PointerButton::Left, true);
        self.pointer_button(PointerButton::Left, false);
    }

    /// Scroll wheel, sent to the widget under the pointer; unhandled events bubble up.
    /// Positive `y` for `Lines` scrolls up, as in winit. `Pixels` are physical pixels.
    pub fn wheel(&mut self, delta: ScrollDelta) {
        self.send(platform::Event::Wheel(delta));
    }

    pub fn set_modifiers(&mut self, m: Modifiers) {
        self.send(platform::Event::ModifiersChanged(m));
    }

    pub fn key(&mut self, key: Key, text: Option<&str>) {
        let ev = |pressed| KeyEvent { key: key.clone(), text: if pressed { text.map(str::to_string) } else { None }, pressed, repeat: false };
        self.send(platform::Event::Key(ev(true)));
        self.send(platform::Event::Key(ev(false)));
    }

    /// Types the string one character at a time, bypassing the IME.
    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            let t = c.to_string();
            self.key(Key::Character(t.clone()), Some(&t));
        }
    }

    pub fn ime(&mut self, ev: ImeEvent) {
        self.send(platform::Event::Ime(ev));
    }

    pub fn ime_allowed(&self) -> bool {
        self.host.ime_allowed
    }

    /// Last IME candidate-window position that was set, in physical pixels.
    pub fn ime_area(&self) -> Option<PixelRect> {
        self.host.ime_area
    }

    pub fn clipboard(&self) -> Option<&str> {
        self.host.clipboard.as_deref()
    }

    pub fn set_clipboard(&mut self, text: &str) {
        self.host.clipboard = Some(text.to_string());
    }

    pub fn next_timer(&self) -> Option<Instant> {
        self.host.wakeup
    }

    /// Moves the virtual clock to `now`, never backwards, firing due timers and frames.
    pub fn fire_timers(&mut self, now: Instant) {
        let now = now.max(self.now());
        self.ui.clock = Clock::Manual(now);
        self.ui.fire_timers(&mut self.host, now);
    }

    /// Animation frame interval, fixed at 60 Hz.
    pub fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    /// Makes the presenter composite the background and glass, as OpenGL presentation does.
    /// The canvas then holds only the UI layer, with a transparent background.
    /// Used to measure paint cost that matches a real window.
    pub fn set_compositing(&mut self, on: bool) {
        self.ui.set_compositing(on);
    }

    pub fn set_window_focused(&mut self, focused: bool) {
        self.send(platform::Event::WindowFocused(focused));
    }

    /// Changes the size (logical pixels) and scale, like moving to another monitor.
    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        let (w, h) = ((width * scale).round() as u32, (height * scale).round() as u32);
        self.send(platform::Event::ScaleFactorChanged { scale: scale as f64 });
        self.send(platform::Event::Resized { width: w, height: h });
    }
}

/// Encodes premultiplied-alpha RGBA8 pixels as PNG.
/// No compression: only for debug screenshots.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for row in rgba.chunks_exact(w * 4).take(h) {
        raw.push(0); // Filter type 0 (none) for this row.
        for px in row.chunks_exact(4) {
            let a = px[3] as u32;
            if a == 255 || a == 0 {
                raw.extend_from_slice(px);
            } else {
                // Premultiplied to straight alpha.
                raw.extend(px[..3].iter().map(|&c| ((c as u32 * 255 + a / 2) / a).min(255) as u8));
                raw.push(px[3]);
            }
        }
    }
    // zlib stored blocks (no compression), each at most 65535 bytes.
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    if chunks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        z.push(chunks.peek().is_none() as u8);
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // Bit depth 8, color type 6 (RGBA).
    for (kind, data) in [(b"IHDR", &ihdr[..]), (b"IDAT", &z[..]), (b"IEND", &[][..])] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
