//! Application entry point: `App` builder, cross-thread `Proxy`, and platform integration.

use std::any::Any;
use std::marker::PhantomData;

use cairn_ui_platform::{self as platform, Blur, Context, Frame, Handler, PixelRect, SceneImage, Surface, WindowConfig};

use crate::element::{El, View};
use crate::host::PlatformHost;
use crate::theme::Theme;
use crate::ui::{Clock, FileDropFn, ThemeFn, Ui, Update, ViewBox};
use crate::{Background, Error, FontSetup, Result, TextSystem};

/// Trait representing a function that generates a UI view from state.
pub trait ViewFn<'a, S: 'static>: 'static {
    type Output: View<S>;
    fn call(&self, state: &'a S) -> Self::Output;
}

impl<'a, S: 'static, F, V> ViewFn<'a, S> for F
where
    F: Fn(&'a S) -> V + 'static,
    V: View<S>,
{
    type Output = V;
    fn call(&self, state: &'a S) -> V {
        self(state)
    }
}

pub(crate) fn erase_view<S: 'static, F>(view: F) -> ViewBox<S>
where
    F: for<'a> ViewFn<'a, S>,
{
    Box::new(move |s: &S| -> El<S> { view.call(s).into() })
}

/// Converts platform system font settings into font configuration for rendering.
pub(crate) fn system_fonts() -> FontSetup {
    let c = platform::font_config();
    FontSetup { dirs: c.dirs, sans_serif: c.families.sans_serif, serif: c.families.serif, monospace: c.families.monospace, locale: c.locale }
}

/// Application builder.
///
/// ```no_run
/// use cairn_ui::prelude::*;
///
/// #[derive(Default)]
/// struct Counter { n: i32 }
///
/// impl Counter {
///     fn view(&self) -> impl View<Self> {
///         column()
///             .padding(16.0)
///             .gap(8.0)
///             .child(label(format!("n = {}", self.n)))
///             .child(button("+1").on_click(|app: &mut Self| app.n += 1))
///     }
/// }
///
/// fn main() -> cairn_ui::Result<()> {
///     App::new(Counter::view).title("counter").run(Counter::default())
/// }
/// ```
pub struct App<S: 'static> {
    view: ViewBox<S>,
    window: WindowConfig,
    theme: Theme,
    file_drop: Option<FileDropFn<S>>,
    theme_source: Option<ThemeFn<S>>,
}

/// File drag-and-drop progress; see `App::on_file_drop`.
#[derive(Clone, Debug, PartialEq)]
pub enum FileDrop {
    /// A file is hovered over the window.
    Hovered(std::path::PathBuf),
    /// A dragged file left the window without being dropped.
    Cancelled,
    /// A file was dropped into the window.
    Dropped(std::path::PathBuf),
}

impl<S: 'static> App<S> {
    pub fn new<F>(view: F) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        App { view: erase_view(view), window: WindowConfig::default(), theme: Theme::default(), file_drop: None, theme_source: None }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.window.title = title.into();
        self
    }

    /// Sets the client area size in logical pixels.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.window.width = width as f64;
        self.window.height = height as f64;
        self
    }

    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.window.min_size = Some((width as f64, height as f64));
        self
    }

    pub fn max_size(mut self, width: f32, height: f32) -> Self {
        self.window.max_size = Some((width as f64, height as f64));
        self
    }

    pub fn maximizable(mut self, maximizable: bool) -> Self {
        self.window.maximizable = maximizable;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.window.resizable = resizable;
        self
    }

    /// Sets whether to use system decorations (title bar and borders); defaults to true.
    /// When disabled, the client area fills the entire window for custom chrome.
    pub fn decorations(mut self, decorations: bool) -> Self {
        self.window.decorations = decorations;
        self
    }

    /// Sets the window icon from RGBA pixel data.
    pub fn icon(mut self, rgba: Vec<u8>, width: u32, height: u32) -> Self {
        self.window.icon = Some(platform::IconData::new(rgba, width, height));
        self
    }

    /// Callback invoked on file drag-and-drop events.
    pub fn on_file_drop(mut self, f: impl Fn(&mut S, FileDrop) + 'static) -> Self {
        self.file_drop = Some(Box::new(f));
        self
    }

    /// Dynamic theme provider called after state changes; rebuilds the UI on `Some`.
    pub fn theme_from(mut self, f: impl Fn(&mut S) -> Option<Theme> + 'static) -> Self {
        self.theme_source = Some(Box::new(f));
        self
    }

    /// Sets the initial application theme; defaults to `Theme::light`.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Opens the window and runs the event loop on the main thread until closed.
    pub fn run(self, state: S) -> Result<()> {
        self.run_with(move |_| state)
    }

    /// Runs the application with deferred state initialization that receives a `Proxy`.
    pub fn run_with(self, init: impl FnOnce(Proxy<S>) -> S + 'static) -> Result<()> {
        // Font scanning can take tens to hundreds of milliseconds; run it in parallel.
        let fonts = std::thread::spawn(|| TextSystem::new(&system_fonts()));
        let resize_border = !self.window.decorations && self.window.resizable;
        let handler = AppHandler {
            pending: Some(Pending {
                view: self.view,
                theme: self.theme,
                init: Box::new(init),
                fonts,
                resize_border,
                file_drop: self.file_drop,
                theme_source: self.theme_source,
            }),
            ui: None,
        };
        platform::run(self.window, handler).map_err(Error::from)
    }
}

/// Thread-safe handle for dispatching state updates from other threads.
pub struct Proxy<S> {
    inner: platform::Proxy,
    _state: PhantomData<fn(S)>,
}

impl<S> Clone for Proxy<S> {
    fn clone(&self) -> Self {
        Proxy { inner: self.inner.clone(), _state: PhantomData }
    }
}

impl<S: 'static> Proxy<S> {
    /// Queues a state update to run on the main thread. Returns false if the window closed.
    pub fn update(&self, f: impl FnOnce(&mut S) + Send + 'static) -> bool {
        let f: Update<S> = Box::new(f);
        self.inner.send(Box::new(f))
    }
}

struct Pending<S: 'static> {
    view: ViewBox<S>,
    theme: Theme,
    init: Box<dyn FnOnce(Proxy<S>) -> S>,
    fonts: std::thread::JoinHandle<TextSystem>,
    resize_border: bool,
    file_drop: Option<FileDropFn<S>>,
    theme_source: Option<ThemeFn<S>>,
}

struct AppHandler<S: 'static> {
    pending: Option<Pending<S>>,
    ui: Option<Ui<S>>,
}

impl<S: 'static> Handler for AppHandler<S> {
    fn init(&mut self, cx: &mut Context) {
        let Some(p) = self.pending.take() else { return };
        let text = p.fonts.join().unwrap_or_else(|_| TextSystem::new(&system_fonts()));
        let state = (p.init)(Proxy { inner: cx.proxy(), _state: PhantomData });
        let mut theme = p.theme;
        let reduce_motion = platform::reduce_motion();
        if reduce_motion {
            theme.motion = theme.motion.reduced();
        }
        let mut ui = Ui::new(state, p.view, theme, text, cx.scale_factor() as f32, cx.physical_size(), Clock::Real);
        ui.set_window_frame(p.resize_border, cx.is_maximized());
        ui.set_file_drop(p.file_drop);
        ui.set_theme_source(p.theme_source, reduce_motion);
        ui.after(&mut PlatformHost(cx));
        self.ui = Some(ui);
    }

    fn event(&mut self, cx: &mut Context, event: platform::Event) {
        if let Some(ui) = &mut self.ui {
            ui.handle(&mut PlatformHost(cx), event);
        }
    }

    fn redraw(&mut self, cx: &mut Context, surface: &mut Surface) {
        let Some(ui) = &mut self.ui else { return };
        let (w, h) = cx.physical_size();
        if (w, h) != (ui.canvas.width(), ui.canvas.height()) {
            let scale = ui.scale();
            ui.set_size(w, h, scale);
        }
        ui.set_compositing(surface.composites());
        let regions = ui.render(&mut PlatformHost(cx));
        let damage: Vec<PixelRect> =
            regions.iter().map(|r| PixelRect { x: r.x.max(0) as u32, y: r.y.max(0) as u32, width: r.width, height: r.height }).collect();
        let canvas = &ui.canvas;
        let style = canvas.backdrop_style();
        let (passes, spread) = style.kernel(ui.scale(), 1.5);
        let frame = Frame {
            width: canvas.width(),
            height: canvas.height(),
            rgba: canvas.pixels(),
            damage: &damage,
            glass: canvas.glass(),
            scene: canvas.scene_image().map(|(rgba, width, height, generation)| SceneImage { rgba, width, height, generation }),
            background: match canvas.background() {
                Background::Solid(c) => [c.r, c.g, c.b, c.a],
                Background::Scene(_) => [0, 0, 0, 255],
            },
            blur: Blur { downsample: style.downsample, passes, spread, saturation: style.saturation, brightness: style.brightness() },
            noise: canvas.noise(),
        };
        if let Err(e) = surface.present(&frame) {
            eprintln!("cairn-ui: {e}");
        }
    }

    fn timer(&mut self, cx: &mut Context) {
        if let Some(ui) = &mut self.ui {
            ui.fire_timers(&mut PlatformHost(cx), std::time::Instant::now());
        }
    }

    fn user_event(&mut self, cx: &mut Context, payload: Box<dyn Any + Send>) {
        if let Some(ui) = &mut self.ui {
            ui.user_event(&mut PlatformHost(cx), payload);
        }
    }
}
