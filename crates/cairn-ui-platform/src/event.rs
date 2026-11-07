//! Input event types, independent of the windowing library. Callers never see winit.

/// Events delivered from the platform layer to the upper layer.
/// Coordinates are always physical pixels; the upper layer converts them with the scale factor.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Client area size changed, in physical pixels.
    Resized { width: u32, height: u32 },
    /// Scale factor changed, from a system scaling change or a move to a monitor with another DPI.
    /// A [`Event::Resized`] usually follows right after.
    ScaleFactorChanged { scale: f64 },
    /// The user clicked the close button.
    /// The upper layer decides whether to call [`crate::Context::exit`].
    CloseRequested,
    WindowFocused(bool),
    /// Pointer moved. The position is relative to the client area's top-left, in physical pixels.
    PointerMoved { x: f64, y: f64 },
    PointerLeft,
    /// A mouse button was pressed or released.
    /// The position is the one from the last [`Event::PointerMoved`].
    PointerButton { button: PointerButton, pressed: bool },
    Wheel(ScrollDelta),
    /// Keyboard key. Keys that the input method consumes during composition
    /// never appear as characters.
    Key(KeyEvent),
    ModifiersChanged(Modifiers),
    /// Input method event. Only received after calling [`crate::Context::set_ime_allowed`]`(true)`.
    Ime(ImeEvent),
    /// A file is dragged over the window but not dropped yet.
    /// One event per file when several are dragged in.
    FileHovered(std::path::PathBuf),
    FileHoverCancelled,
    /// A file was dropped into the window. One event per file when several are dropped.
    FileDropped(std::path::PathBuf),
}

/// Edge or corner dragged to resize the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// In lines, as from a normal mouse wheel.
    Lines { x: f32, y: f32 },
    /// In physical pixels, as from a touchpad and similar devices.
    Pixels { x: f64, y: f64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    /// Text produced by this key press, after Shift, case and dead keys are applied.
    /// Usually `None` for control keys.
    pub text: Option<String>,
    pub pressed: bool,
    pub repeat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Named(NamedKey),
    /// Character key. The text is the character under the current layout, unaffected by Ctrl.
    Character(String),
    Unidentified,
}

/// The named keys the upper layer recognizes. Keys not listed map to [`NamedKey::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Escape,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Shift,
    Control,
    Alt,
    Super,
    /// On Windows, a key handled by the IME (VK_PROCESSKEY). Text boxes should ignore it.
    Process,
    F(u8),
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Windows key on Windows, Command key on macOS.
    pub logo: bool,
}

impl Modifiers {
    /// The platform's usual shortcut modifier: Ctrl on Windows and Linux, Command on macOS.
    pub fn shortcut(&self) -> bool {
        if cfg!(target_vendor = "apple") { self.logo } else { self.ctrl }
    }

    /// Modifier for moving the cursor by word: Ctrl on Windows and Linux, Option on macOS.
    pub fn word_jump(&self) -> bool {
        if cfg!(target_vendor = "apple") { self.alt } else { self.ctrl }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// Composition started. On Windows this is sent before every composition.
    /// It does not mean that the focus changed.
    Enabled,
    /// Preedit text updated. An empty `text` clears the preedit.
    /// `cursor` is a byte range inside the preedit text, the clause or caret the IME has selected.
    /// `None` hides the cursor.
    Preedit { text: String, cursor: Option<(usize, usize)> },
    Commit(String),
    /// Composition ended. Any leftover preedit text should be cleared.
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorIcon {
    #[default]
    Default,
    /// Hand shape, for clickable elements.
    Pointer,
    /// I-beam shape, for text.
    Text,
    NotAllowed,
    ResizeHorizontal,
    ResizeVertical,
    /// Top-left to bottom-right double arrow.
    ResizeNwSe,
    /// Top-right to bottom-left double arrow.
    ResizeNeSw,
}
