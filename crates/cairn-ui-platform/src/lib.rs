//! Platform abstraction layer for `cairn-ui`.
//!
//! Encapsulates OS-specific functionality including windows, event loops (winit), presentation
//! (OpenGL or softbuffer), IME, DPI scaling, clipboard, and system settings. Upper layers use types
//! defined here without directly depending on winit, glutin, softbuffer, or platform APIs.
//!
//! Implement [`Handler`] and call [`run`] to start the application.
//!
//! # Environment Variables
//!
//! - `CAIRN_UI_SCALE_FACTOR=1.5`: Forces a fixed DPI scale factor for testing.
//! - `CAIRN_UI_SOFTWARE=1`: Forces software presentation, bypassing OpenGL.

mod convert;
mod event;
#[cfg(windows)]
mod gl;
mod present;
mod sys;

pub use event::*;
pub use present::{Blur, Frame, SceneImage, Surface};

use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// Integer rectangle in physical pixel coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Clamps the rectangle to the given `width` and `height` bounds.
    pub fn clamp(&self, width: u32, height: u32) -> PixelRect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        PixelRect { x, y, width: self.x.saturating_add(self.width).min(width) - x, height: self.y.saturating_add(self.height).min(height) - y }
    }
}

/// Window configuration. Dimensions are in logical pixels.
#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub min_size: Option<(f64, f64)>,
    pub max_size: Option<(f64, f64)>,
    pub resizable: bool,
    pub maximizable: bool,
    /// Enables system title bar and borders. When disabled, the client area fills the window while
    /// preserving system shadows and Windows 11 rounded corners.
    pub decorations: bool,
    pub icon: Option<IconData>,
}

/// Raw RGBA window icon pixels and dimensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl IconData {
    pub fn new(rgba: Vec<u8>, width: u32, height: u32) -> Self {
        Self { rgba, width, height }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            title: "cairn-ui".into(),
            width: 640.0,
            height: 480.0,
            min_size: None,
            max_size: None,
            resizable: true,
            maximizable: true,
            decorations: true,
            icon: None,
        }
    }
}

/// Application event handler. All callbacks run on the main thread.
pub trait Handler: 'static {
    /// Called once after window creation, before the first event or redraw.
    fn init(&mut self, cx: &mut Context);
    /// Handles input and window events.
    fn event(&mut self, cx: &mut Context, event: Event);
    /// Presents a new frame when the OS requests a repaint or [`Context::request_redraw`] was called.
    fn redraw(&mut self, cx: &mut Context, surface: &mut Surface);
    /// Called when the timer configured via [`Context::set_wakeup`] expires.
    fn timer(&mut self, _cx: &mut Context) {}
    /// Called when a message sent via [`Proxy::send`] is received on the main thread.
    fn user_event(&mut self, _cx: &mut Context, _payload: Box<dyn Any + Send>) {}
}

/// Platform abstraction error.
#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    EventLoop(winit::error::EventLoopError),
    Window(winit::error::OsError),
    Surface(softbuffer::SoftBufferError),
    Gl(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::EventLoop(e) => write!(f, "事件循环错误: {e}"),
            ErrorKind::Window(e) => write!(f, "创建窗口失败: {e}"),
            ErrorKind::Surface(e) => write!(f, "上屏失败: {e}"),
            ErrorKind::Gl(e) => write!(f, "OpenGL 上屏失败: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<softbuffer::SoftBufferError> for Error {
    fn from(e: softbuffer::SoftBufferError) -> Self {
        Error(ErrorKind::Surface(e))
    }
}

type UserPayload = Box<dyn Any + Send>;

/// Cross-thread proxy to wake the event loop and post data to the main thread.
#[derive(Clone)]
pub struct Proxy(EventLoopProxy<UserPayload>);

impl Proxy {
    /// Posts data to the main thread's [`Handler::user_event`]. Returns `false` if the event loop exited.
    pub fn send(&self, payload: Box<dyn Any + Send>) -> bool {
        self.0.send_event(payload).is_ok()
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Proxy")
    }
}

/// Window control context available during handler callbacks.
pub struct Context<'a> {
    window: &'a Window,
    event_loop: &'a ActiveEventLoop,
    shared: &'a mut Shared,
}

impl Context<'_> {
    /// Current DPI scale factor (physical pixels per logical pixel).
    pub fn scale_factor(&self) -> f64 {
        self.shared.scale_override.unwrap_or_else(|| self.window.scale_factor())
    }

    /// Physical size of the client area. May be zero when minimized.
    pub fn physical_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// Requests a [`Handler::redraw`]. Multiple requests in the same cycle are coalesced.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Refresh interval of the current monitor, defaulting to 60 Hz if unavailable.
    pub fn frame_interval(&self) -> Duration {
        let mhz = self.window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).filter(|&m| m >= 20_000).unwrap_or(60_000);
        Duration::from_nanos(1_000_000_000_000 / mhz as u64)
    }

    pub fn set_cursor(&mut self, icon: CursorIcon) {
        if self.shared.cursor != icon {
            self.shared.cursor = icon;
            self.window.set_cursor(convert::cursor_icon(icon));
        }
    }

    /// Enables or disables input method editor (IME) interactions.
    pub fn set_ime_allowed(&mut self, allowed: bool) {
        if self.shared.ime_allowed != allowed {
            self.shared.ime_allowed = allowed;
            self.shared.ime_area = None;
            self.window.set_ime_allowed(allowed);
        }
    }

    /// Sets the IME candidate window position in physical pixel coordinates.
    pub fn set_ime_cursor_area(&mut self, area: PixelRect) {
        if self.shared.ime_area != Some(area) {
            self.shared.ime_area = Some(area);
            self.window.set_ime_cursor_area(PhysicalPosition::new(area.x as i32, area.y as i32), PhysicalSize::new(area.width.max(1), area.height.max(1)));
        }
    }

    /// Schedules a wake-up time for [`Handler::timer`], replacing any previous deadline.
    pub fn set_wakeup(&mut self, at: Option<Instant>) {
        self.shared.wakeup = at;
    }

    pub fn exit(&self) {
        self.event_loop.exit();
    }

    /// Initiates window move drag using the OS move loop. Called on mouse down.
    pub fn drag_window(&self) {
        let _ = self.window.drag_window();
    }

    /// Initiates interactive window resize on the specified edge or corner.
    pub fn drag_resize_window(&self, edge: ResizeEdge) {
        let _ = self.window.drag_resize_window(convert::resize_direction(edge));
    }

    pub fn minimize(&self) {
        self.window.set_minimized(true);
    }

    pub fn is_maximized(&self) -> bool {
        self.window.is_maximized()
    }

    pub fn is_maximizable(&self) -> bool {
        self.shared.maximizable
    }

    /// Toggles between maximized and restored window states.
    pub fn toggle_maximize(&self) {
        if self.shared.maximizable {
            self.window.set_maximized(!self.window.is_maximized());
        }
    }

    /// Updates the window icon dynamically.
    pub fn set_window_icon(&self, icon: Option<IconData>) {
        let winit_icon = icon.and_then(|ic| winit::window::Icon::from_rgba(ic.rgba, ic.width, ic.height).ok());
        self.window.set_window_icon(winit_icon);
    }

    pub fn proxy(&self) -> Proxy {
        Proxy(self.shared.proxy.clone())
    }

    pub fn clipboard_get(&self) -> Option<String> {
        sys::clipboard_get(self.window)
    }

    pub fn clipboard_set(&self, text: &str) -> bool {
        sys::clipboard_set(self.window, text)
    }
}

/// Font discovery configuration including font directories and preferred family fallbacks.
#[derive(Clone, Debug)]
pub struct FontConfig {
    pub dirs: Vec<PathBuf>,
    pub families: FontFamilies,
    /// BCP 47 language tag (e.g. `zh-CN`) used to disambiguate CJK font fallback.
    pub locale: String,
}

/// Priority list of concrete font names mapped from generic font families.
#[derive(Clone, Debug, Default)]
pub struct FontFamilies {
    pub sans_serif: Vec<String>,
    pub serif: Vec<String>,
    pub monospace: Vec<String>,
}

/// Returns the current system UI locale (e.g. `zh-CN`).
pub fn locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string())
}

/// Returns the default font directories and preferred font families on the host system.
pub fn font_config() -> FontConfig {
    let locale = locale();
    FontConfig { dirs: sys::font_dirs(), families: sys::default_font_families(&locale), locale }
}

/// Caret blink interval, or `None` if caret blinking is disabled.
pub fn caret_blink_interval() -> Option<Duration> {
    sys::caret_blink_interval()
}

/// Maximum time interval between clicks to register a double click.
pub fn double_click_interval() -> Duration {
    sys::double_click_interval()
}

/// Maximum movement distance in physical pixels to register a double click.
pub fn double_click_distance() -> f64 {
    sys::double_click_distance()
}

/// Whether system animations are disabled in OS accessibility settings.
pub fn reduce_motion() -> bool {
    sys::reduce_motion()
}

/// Creates the window and runs the event loop until [`Context::exit`] is called. Must run on main thread.
pub fn run<H: Handler>(config: WindowConfig, handler: H) -> Result<(), Error> {
    let event_loop = EventLoop::<UserPayload>::with_user_event().build().map_err(|e| Error(ErrorKind::EventLoop(e)))?;
    let scale_override =
        std::env::var("CAIRN_UI_SCALE_FACTOR").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s >= 0.5 && *s <= 4.0);
    let maximizable = config.maximizable;
    let mut runner = Runner {
        config,
        handler,
        window: None,
        error: None,
        shared: Shared {
            wakeup: None,
            scale_override,
            proxy: event_loop.create_proxy(),
            cursor: CursorIcon::Default,
            ime_allowed: false,
            ime_area: None,
            maximizable,
        },
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut runner).map_err(|e| Error(ErrorKind::EventLoop(e)))?;
    match runner.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct Shared {
    wakeup: Option<Instant>,
    scale_override: Option<f64>,
    proxy: EventLoopProxy<UserPayload>,
    cursor: CursorIcon,
    ime_allowed: bool,
    ime_area: Option<PixelRect>,
    maximizable: bool,
}

struct WindowState {
    // Drop order matters: presentation surface must be dropped before the window.
    surface: Surface,
    window: Rc<Window>,
}

struct Runner<H> {
    config: WindowConfig,
    handler: H,
    window: Option<WindowState>,
    shared: Shared,
    error: Option<Error>,
}

impl<H: Handler> Runner<H> {
    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<WindowState, Error> {
        let c = &self.config;
        let mut attrs = Window::default_attributes()
            .with_title(c.title.clone())
            .with_resizable(c.resizable)
            .with_decorations(c.decorations)
            // Start hidden to prevent white flashes before the first frame is painted.
            .with_visible(false);
        match self.shared.scale_override {
            Some(s) => {
                attrs = attrs.with_inner_size(PhysicalSize::new(c.width * s, c.height * s));
                if let Some((w, h)) = c.min_size {
                    attrs = attrs.with_min_inner_size(PhysicalSize::new(w * s, h * s));
                }
                if let Some((w, h)) = c.max_size {
                    attrs = attrs.with_max_inner_size(PhysicalSize::new(w * s, h * s));
                }
            }
            None => {
                attrs = attrs.with_inner_size(LogicalSize::new(c.width, c.height));
                if let Some((w, h)) = c.min_size {
                    attrs = attrs.with_min_inner_size(LogicalSize::new(w, h));
                }
                if let Some((w, h)) = c.max_size {
                    attrs = attrs.with_max_inner_size(LogicalSize::new(w, h));
                }
            }
        }
        if !c.maximizable {
            attrs = attrs.with_enabled_buttons(winit::window::WindowButtons::CLOSE | winit::window::WindowButtons::MINIMIZE);
        }
        if let Some(icon) = &c.icon {
            if let Ok(winit_icon) = winit::window::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height) {
                attrs = attrs.with_window_icon(Some(winit_icon));
            }
        }
        if !c.decorations {
            attrs = sys::undecorated(attrs);
        }
        let mut window = None;
        #[cfg(windows)]
        if std::env::var_os("CAIRN_UI_SOFTWARE").is_none() {
            match gl::create(event_loop, attrs.clone()) {
                Ok((w, gl)) => {
                    return Ok(WindowState { surface: Surface::gl(gl), window: Rc::new(w) });
                }
                Err((w, e)) => {
                    eprintln!("cairn-ui: 无法使用 OpenGL（{e}），改用软件上屏");
                    window = w;
                }
            }
        }
        let window = match window {
            Some(w) => Rc::new(w),
            None => Rc::new(event_loop.create_window(attrs).map_err(|e| Error(ErrorKind::Window(e)))?),
        };
        let context = softbuffer::Context::new(window.clone())?;
        let surface = Surface::soft(context, window.clone())?;
        Ok(WindowState { surface, window })
    }

    fn with_cx(&mut self, event_loop: &ActiveEventLoop, f: impl FnOnce(&mut H, &mut Context, &mut Surface)) {
        let Some(state) = self.window.as_mut() else { return };
        let mut cx = Context { window: &state.window, event_loop, shared: &mut self.shared };
        f(&mut self.handler, &mut cx, &mut state.surface);
    }

    fn forward(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        self.with_cx(event_loop, |h, cx, _| h.event(cx, event));
    }
}

impl<H: Handler> ApplicationHandler<UserPayload> for Runner<H> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        // The deadline may have passed even under other start causes if event handling took long,
        // or clock jitter caused ResumeTimeReached to fire slightly early.
        let timer_due = match (cause, self.shared.wakeup) {
            (StartCause::ResumeTimeReached { .. }, Some(_)) => true,
            (_, Some(at)) => Instant::now() + std::time::Duration::from_millis(1) >= at,
            _ => false,
        };
        if timer_due {
            self.shared.wakeup = None;
            self.with_cx(event_loop, |h, cx, _| h.timer(cx));
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match self.create_window(event_loop) {
            Ok(state) => self.window = Some(state),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        self.with_cx(event_loop, |h, cx, surface| {
            h.init(cx);
            h.redraw(cx, surface);
        });
        if let Some(state) = &self.window {
            state.window.set_visible(true);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, payload: UserPayload) {
        self.with_cx(event_loop, |h, cx, _| h.user_event(cx, payload));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Windows reports a dummy client size (e.g. 160x28) while minimized. Avoid layout and rendering
        // so restore animations do not stretch that dummy frame.
        let minimized = self.window.as_ref().is_some_and(|s| s.window.is_minimized() == Some(true));
        if minimized && matches!(event, WindowEvent::RedrawRequested | WindowEvent::Resized(_)) {
            return;
        }
        let ev = match event {
            WindowEvent::RedrawRequested => {
                self.with_cx(event_loop, |h, cx, surface| h.redraw(cx, surface));
                return;
            }
            WindowEvent::Resized(size) => Event::Resized { width: size.width, height: size.height },
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if self.shared.scale_override.is_some() {
                    return;
                }
                Event::ScaleFactorChanged { scale: scale_factor }
            }
            WindowEvent::CloseRequested => Event::CloseRequested,
            WindowEvent::Focused(f) => Event::WindowFocused(f),
            WindowEvent::CursorMoved { position, .. } => Event::PointerMoved { x: position.x, y: position.y },
            WindowEvent::CursorLeft { .. } => Event::PointerLeft,
            WindowEvent::MouseInput { state, button, .. } => {
                Event::PointerButton { button: convert::pointer_button(button), pressed: state == ElementState::Pressed }
            }
            WindowEvent::MouseWheel { delta, .. } => Event::Wheel(convert::scroll_delta(delta)),
            // Synthetic events are synthesized by winit when focus is regained; ignore them as input.
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => Event::Key(convert::key_event(&event)),
            WindowEvent::ModifiersChanged(m) => Event::ModifiersChanged(convert::modifiers(&m)),
            WindowEvent::Ime(ime) => Event::Ime(convert::ime(ime)),
            WindowEvent::HoveredFile(path) => Event::FileHovered(path),
            WindowEvent::HoveredFileCancelled => Event::FileHoverCancelled,
            WindowEvent::DroppedFile(path) => Event::FileDropped(path),
            _ => return,
        };
        self.forward(event_loop, ev);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(match self.shared.wakeup {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
