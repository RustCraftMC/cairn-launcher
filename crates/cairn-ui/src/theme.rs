//! Design tokens: colors, type, radii, spacing, shadows and motion for all widgets.
//! Widget code reads appearance values from here, never from literals.
//! Built-in themes: [`Theme::light`] (default) and [`Theme::nocturne`] (night glass).
//!
//! Change fields of a built-in theme to make your own:
//!
//! ```
//! use cairn_ui::{Color, Theme};
//! let mut theme = Theme::nocturne();
//! theme.radius.panel = 12.0;
//! theme.panel.fill = Color::rgbaf(255, 255, 255, 0.06);
//! ```

use std::rc::Rc;
use std::time::Duration;

use crate::anim::Easing;
use crate::{BackdropStyle, Color, Insets, NightScene, Scene, Shadow};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    /// Font size, in logical pixels.
    pub size: f32,
    /// Font weight: 400 regular, 600 semibold, 700 bold.
    pub weight: u16,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub title: TypeStyle,
    /// Body text. Also the default size for labels, buttons and inputs.
    pub body: TypeStyle,
    pub secondary: TypeStyle,
    pub caption: TypeStyle,
    /// Line height, as a multiple of the font size.
    pub line_height: f32,
}

/// Corner radii. Icon buttons and sliders are fully round, so they are not listed here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub panel: f32,
    pub bar: f32,
    pub control: f32,
    pub tag: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub panel_padding: f32,
    pub gap: f32,
}

/// Panel. In `nocturne` it is glass (blurred backdrop, translucent white, inner glow,
/// hairline edge, layered shadows). In `light` it is a white card.
///
/// All edges are 1 physical pixel: top inner highlight, bottom inner dark edge, border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelStyle {
    /// Paints the blurred backdrop first (`crate::Brush::Backdrop`).
    pub backdrop: bool,
    pub fill: Color,
    pub fill_hover: Color,
    /// Inner glow: fades from this color at the top to transparent at the bottom.
    /// Transparent means none.
    pub glow: Color,
    pub top_highlight: Color,
    pub bottom_edge: Color,
    pub border: Color,
    pub border_hover: Color,
    pub shadow: Shadow,
    pub shadow_hover: Shadow,
    /// Distance the panel lifts on hover, in logical pixels.
    pub lift: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonStyle {
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_pressed: Color,
    /// 1 physical pixel border. Transparent means none.
    pub border: Color,
    pub border_hover: Color,
    pub text: Color,
    pub weight: u16,
    pub top_highlight: Color,
    pub glow: Shadow,
    pub glow_hover: Shadow,
    pub lift: f32,
    pub press_scale: f32,
    pub padding: Insets,
    pub icon_size: f32,
    pub icon_gap: f32,
    pub disabled_fill: Color,
    pub disabled_text: Color,
    pub disabled_border: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconButtonStyle {
    /// Diameter.
    pub size: f32,
    pub fill: Color,
    pub fill_hover: Color,
    pub border: Color,
    pub border_hover: Color,
    pub top_highlight: Color,
    pub icon: Color,
    pub icon_size: f32,
    pub press_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputStyle {
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_focus: Color,
    pub border: Color,
    pub border_hover: Color,
    pub border_focus: Color,
    /// Border width in logical pixels. 0 or less means 1 physical pixel.
    pub border_width: f32,
    pub border_focus_width: f32,
    /// Outer glow when focused. Transparent means none.
    pub focus_glow: Shadow,
    pub text: Color,
    pub placeholder: Color,
    pub icon: Color,
    pub icon_size: f32,
    pub icon_gap: f32,
    pub caret: Color,
    pub selection: Color,
    /// Underline of IME preedit text.
    pub preedit: Color,
    pub padding: Insets,
}

/// Scrollbar and wheel feel for scroll containers (`crate::scroll`).
/// The scrollbar is a thin thumb drawn over the content, with no track background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollStyle {
    /// Thumb width in logical pixels at rest (see `bar_width_hover` for hover and drag).
    pub bar_width: f32,
    pub bar_width_hover: f32,
    /// Distance of the thumb from the viewport's right, top and bottom edges.
    pub bar_inset: f32,
    /// Minimum thumb length, so long content never shrinks the thumb to a dot.
    pub min_thumb: f32,
    /// Thumb colors: at rest, pointer in the scroll area, and over the bar or dragging.
    pub thumb: Color,
    pub thumb_hover: Color,
    pub thumb_active: Color,
    /// Distance scrolled per wheel notch, in logical pixels.
    pub wheel_step: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStyle {
    pub track_height: f32,
    pub track: Color,
    pub track_border: Color,
    pub fill: Color,
    /// Diameter.
    pub knob_size: f32,
    pub knob: Color,
    pub knob_border: Color,
    pub knob_shadow: Shadow,
    /// Thin ring around the knob while dragging: color, width, and gap to the knob.
    pub knob_ring: Color,
    pub knob_ring_width: f32,
    pub knob_ring_gap: f32,
}

/// Window buttons drawn by the app (`crate::window_controls`):
/// minimize, maximize or restore, and close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionStyle {
    /// Width and height of each button.
    pub width: f32,
    pub height: f32,
    pub icon: Color,
    pub icon_hover: Color,
    pub icon_size: f32,
    pub fill_hover: Color,
    pub fill_pressed: Color,
    /// Close button fill and icon color when hovered or pressed.
    pub close_fill_hover: Color,
    pub close_fill_pressed: Color,
    pub close_icon_hover: Color,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckboxStyle {
    pub size: f32,
    pub radius: f32,
    pub fill: Color,
    pub fill_hover: Color,
    /// 1 physical pixel border.
    pub border: Color,
    pub border_hover: Color,
    pub checked_fill: Color,
    pub checked_fill_hover: Color,
    pub check: Color,
    pub label_gap: f32,
}

/// Tab, also used as a selectable toggle button (`crate::tab`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStyle {
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_selected: Color,
    /// 1 physical pixel border when selected. Transparent means none.
    pub border_selected: Color,
    pub text: Color,
    pub text_hover: Color,
    pub text_selected: Color,
    pub weight: u16,
    pub icon_size: f32,
    pub gap: f32,
    pub padding: Insets,
    /// Number badge (e.g. item count): text and fill, one set unselected, one selected.
    pub badge_text: Color,
    pub badge_fill: Color,
    pub badge_text_selected: Color,
    pub badge_fill_selected: Color,
}

/// Progress bar (`crate::progress_bar`) and loading spinner (`crate::spinner`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressStyle {
    pub height: f32,
    pub track: Color,
    /// Completed part: a horizontal gradient from `fill` to `fill_end`.
    pub fill: Color,
    pub fill_end: Color,
    /// Segment that moves while progress is unknown: width as a fraction of the track,
    /// and the duration of one round trip.
    pub indeterminate_width: f32,
    pub indeterminate_period: Duration,
    /// Loading spinner: diameter, color, and duration of one turn.
    pub spinner_size: f32,
    pub spinner: Color,
    pub spinner_period: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListItemStyle {
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_pressed: Color,
    pub fill_selected: Color,
    /// 1 physical pixel border when selected. Transparent means none.
    pub border_selected: Color,
    pub padding: Insets,
    pub gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuStyle {
    /// Outer frame radius and padding. Item radius is the difference between them,
    /// so items stay concentric with the frame.
    pub radius: f32,
    pub inset: f32,
    pub item_padding: Insets,
    pub icon_size: f32,
    pub gap: f32,
    pub text: Color,
    pub fill_hover: Color,
    pub text_hover: Color,
    /// Text color for dangerous actions such as delete, also used as the fill on hover.
    pub danger: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TagStyle {
    pub fill: Color,
    pub text: Color,
    pub accent_fill: Color,
    pub accent_text: Color,
    pub font_size: f32,
    pub padding: Insets,
}

/// Keyboard focus ring: a rounded frame `width` wide, drawn `offset` outside the control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusRing {
    pub color: Color,
    pub width: f32,
    pub offset: f32,
}

/// Motion durations and curves. When the system has animations turned off (read by
/// `crate::App` at startup), all durations become 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub hover: Duration,
    pub press: Duration,
    pub focus: Duration,
    /// Smooth wheel scrolling. Zero jumps straight to the new position.
    pub scroll: Duration,
    pub progress: Duration,
    /// Appear and leave animations (`crate::reveal`): duration, and the distance an item
    /// moves in from, in logical pixels.
    pub enter: Duration,
    pub exit: Duration,
    pub enter_offset: f32,
    /// Expand and collapse (`crate::collapse`): time to grow to full height.
    /// Collapsing takes 75% of this.
    pub expand: Duration,
    pub easing: Easing,
    pub press_easing: Easing,
}

impl Motion {
    pub fn reduced(self) -> Motion {
        Motion {
            hover: Duration::ZERO,
            press: Duration::ZERO,
            focus: Duration::ZERO,
            scroll: Duration::ZERO,
            progress: Duration::ZERO,
            enter: Duration::ZERO,
            exit: Duration::ZERO,
            expand: Duration::ZERO,
            ..self
        }
    }
}

/// Global appearance, set with `crate::App::theme`.
#[derive(Clone, Debug)]
pub struct Theme {
    /// Window background color. When `Theme::scene` is set, it is also the fallback color
    /// for the backdrop blur before the scene is generated.
    pub background: Color,
    /// Window background scene. When `None`, the window uses the plain `background` color.
    pub scene: Option<Rc<dyn Scene>>,
    pub backdrop: BackdropStyle,
    /// Opacity of the global noise (0 turns it off). It covers the whole frame and also
    /// removes banding in dark gradients.
    pub noise: f32,
    pub typography: Typography,
    /// The only accent color: focus and selection.
    pub accent: Color,
    pub highlight: Color,
    pub radius: Radii,
    pub spacing: Spacing,
    pub focus_ring: FocusRing,
    pub panel: PanelStyle,
    pub bar: PanelStyle,
    /// Popover (`crate::Panel::popover`): a small panel dropped from a button.
    /// More opaque and with a heavier shadow, so it stays readable over other panels.
    pub popover: PanelStyle,
    pub divider: Color,
    pub button: ButtonStyle,
    /// Accent button (`crate::Button::accent`). Use at most one or two per page.
    pub accent_button: ButtonStyle,
    pub icon_button: IconButtonStyle,
    pub input: InputStyle,
    pub scroll: ScrollStyle,
    pub slider: SliderStyle,
    pub caption: CaptionStyle,
    pub checkbox: CheckboxStyle,
    pub tab: TabStyle,
    pub progress: ProgressStyle,
    pub list_item: ListItemStyle,
    pub menu: MenuStyle,
    pub tag: TagStyle,
    /// Opacity when disabled, for controls with no disabled colors of their own
    /// (icon buttons, sliders).
    pub disabled_opacity: f32,
    pub motion: Motion,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}

impl Theme {
    pub fn light() -> Self {
        let text = Color::hex(0x1E2024);
        let muted = Color::hex(0x8A9099);
        let accent = Color::hex(0x2B66D9);
        let border = Color::hex(0xC5CAD2);
        let border_hover = Color::hex(0x9EA5B0);
        let button = ButtonStyle {
            fill: accent,
            fill_hover: Color::hex(0x3B76E6),
            fill_pressed: Color::hex(0x2152B5),
            border: Color::TRANSPARENT,
            border_hover: Color::TRANSPARENT,
            text: Color::WHITE,
            weight: 400,
            top_highlight: Color::TRANSPARENT,
            glow: Shadow::NONE,
            glow_hover: Shadow::NONE,
            lift: 0.0,
            press_scale: 1.0,
            padding: Insets::symmetric(16.0, 7.0),
            icon_size: 16.0,
            icon_gap: 7.0,
            disabled_fill: Color::hex(0xDADDE2),
            disabled_text: Color::hex(0x9AA0A8),
            disabled_border: Color::TRANSPARENT,
        };
        Theme {
            background: Color::hex(0xF4F5F7),
            scene: None,
            backdrop: BackdropStyle::default(),
            noise: 0.0,
            typography: Typography {
                title: TypeStyle { size: 20.0, weight: 600, color: text },
                body: TypeStyle { size: 14.0, weight: 400, color: text },
                secondary: TypeStyle { size: 14.0, weight: 400, color: Color::hex(0x5D636B) },
                caption: TypeStyle { size: 12.0, weight: 400, color: muted },
                line_height: 1.45,
            },
            accent,
            highlight: accent,
            radius: Radii { panel: 10.0, bar: 10.0, control: 6.0, tag: 4.0 },
            spacing: Spacing { panel_padding: 16.0, gap: 10.0 },
            focus_ring: FocusRing { color: accent, width: 2.0, offset: 3.0 },
            panel: PanelStyle {
                backdrop: false,
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                glow: Color::TRANSPARENT,
                top_highlight: Color::TRANSPARENT,
                bottom_edge: Color::TRANSPARENT,
                border: Color::hex(0xE1E4E8),
                border_hover: Color::hex(0xD0D5DB),
                shadow: Shadow::new(0.0, 1.0, 3.0, Color::rgbaf(16, 24, 40, 0.08)),
                shadow_hover: Shadow::new(0.0, 4.0, 12.0, Color::rgbaf(16, 24, 40, 0.10)),
                lift: 0.0,
            },
            bar: PanelStyle {
                backdrop: false,
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                glow: Color::TRANSPARENT,
                top_highlight: Color::TRANSPARENT,
                bottom_edge: Color::TRANSPARENT,
                border: Color::TRANSPARENT,
                border_hover: Color::TRANSPARENT,
                shadow: Shadow::new(0.0, 4.0, 16.0, Color::rgbaf(16, 24, 40, 0.12)),
                shadow_hover: Shadow::new(0.0, 4.0, 16.0, Color::rgbaf(16, 24, 40, 0.12)),
                lift: 0.0,
            },
            divider: Color::hex(0xE1E4E8),
            popover: PanelStyle {
                backdrop: false,
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                glow: Color::TRANSPARENT,
                top_highlight: Color::TRANSPARENT,
                bottom_edge: Color::TRANSPARENT,
                border: Color::hex(0xD0D5DB),
                border_hover: Color::hex(0xD0D5DB),
                shadow: Shadow::new(0.0, 8.0, 24.0, Color::rgbaf(16, 24, 40, 0.18)),
                shadow_hover: Shadow::new(0.0, 8.0, 24.0, Color::rgbaf(16, 24, 40, 0.18)),
                lift: 0.0,
            },
            button,
            accent_button: button,
            icon_button: IconButtonStyle {
                size: 32.0,
                fill: Color::rgbaf(30, 32, 36, 0.05),
                fill_hover: Color::rgbaf(30, 32, 36, 0.09),
                border: Color::hex(0xD0D5DB),
                border_hover,
                top_highlight: Color::TRANSPARENT,
                icon: accent,
                icon_size: 16.0,
                press_scale: 1.0,
            },
            input: InputStyle {
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                fill_focus: Color::WHITE,
                border,
                border_hover,
                border_focus: accent,
                border_width: 1.0,
                border_focus_width: 2.0,
                focus_glow: Shadow::NONE,
                text,
                placeholder: muted,
                icon: muted,
                icon_size: 16.0,
                icon_gap: 8.0,
                caret: text,
                selection: accent.with_alpha(72),
                preedit: text,
                padding: Insets::symmetric(10.0, 7.0),
            },
            scroll: ScrollStyle {
                bar_width: 6.0,
                bar_width_hover: 8.0,
                bar_inset: 3.0,
                min_thumb: 32.0,
                thumb: Color::rgbaf(30, 32, 36, 0.22),
                thumb_hover: Color::rgbaf(30, 32, 36, 0.35),
                thumb_active: Color::rgbaf(30, 32, 36, 0.5),
                wheel_step: 56.0,
            },
            slider: SliderStyle {
                track_height: 4.0,
                track: Color::hex(0xDADDE2),
                track_border: Color::TRANSPARENT,
                fill: accent,
                knob_size: 16.0,
                knob: Color::WHITE,
                knob_border: border,
                knob_shadow: Shadow::new(0.0, 1.0, 3.0, Color::rgbaf(0, 0, 0, 0.18)),
                knob_ring: accent,
                knob_ring_width: 2.0,
                knob_ring_gap: 2.0,
            },
            caption: CaptionStyle {
                width: 46.0,
                height: 32.0,
                icon: text,
                icon_hover: text,
                icon_size: 10.0,
                fill_hover: Color::rgbaf(30, 32, 36, 0.08),
                fill_pressed: Color::rgbaf(30, 32, 36, 0.14),
                close_fill_hover: Color::hex(0xC42B1C),
                close_fill_pressed: Color::hex(0xB12A1C),
                close_icon_hover: Color::WHITE,
                radius: 0.0,
            },
            checkbox: CheckboxStyle {
                size: 16.0,
                radius: 4.0,
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                border,
                border_hover,
                checked_fill: accent,
                checked_fill_hover: Color::hex(0x3B76E6),
                check: Color::WHITE,
                label_gap: 8.0,
            },
            tab: TabStyle {
                fill: Color::TRANSPARENT,
                fill_hover: Color::rgbaf(30, 32, 36, 0.05),
                fill_selected: Color::rgbaf(43, 102, 217, 0.10),
                border_selected: Color::TRANSPARENT,
                text: Color::hex(0x5D636B),
                text_hover: text,
                text_selected: accent,
                weight: 500,
                icon_size: 16.0,
                gap: 6.0,
                padding: Insets::symmetric(12.0, 6.0),
                badge_text: Color::hex(0x5D636B),
                badge_fill: Color::rgbaf(30, 32, 36, 0.07),
                badge_text_selected: accent,
                badge_fill_selected: Color::rgbaf(43, 102, 217, 0.12),
            },
            progress: ProgressStyle {
                height: 4.0,
                track: Color::hex(0xDADDE2),
                fill: accent,
                fill_end: accent,
                indeterminate_width: 0.3,
                indeterminate_period: Duration::from_millis(1400),
                spinner_size: 16.0,
                spinner: accent,
                spinner_period: Duration::from_millis(900),
            },
            list_item: ListItemStyle {
                fill: Color::TRANSPARENT,
                fill_hover: Color::rgbaf(30, 32, 36, 0.04),
                fill_pressed: Color::rgbaf(30, 32, 36, 0.08),
                fill_selected: Color::rgbaf(43, 102, 217, 0.08),
                border_selected: Color::TRANSPARENT,
                padding: Insets::symmetric(10.0, 8.0),
                gap: 10.0,
            },
            menu: MenuStyle {
                radius: 8.0,
                inset: 4.0,
                item_padding: Insets::symmetric(10.0, 5.0),
                icon_size: 15.0,
                gap: 9.0,
                text,
                fill_hover: accent,
                text_hover: Color::WHITE,
                danger: Color::hex(0xD93636),
            },
            tag: TagStyle {
                fill: Color::rgbaf(30, 32, 36, 0.07),
                text: Color::hex(0x5D636B),
                accent_fill: Color::rgbaf(43, 102, 217, 0.12),
                accent_text: accent,
                font_size: 12.0,
                padding: Insets::symmetric(7.0, 1.0),
            },
            disabled_opacity: 0.5,
            // State changes are instant, with no animation frames.
            motion: Motion {
                hover: Duration::ZERO,
                press: Duration::ZERO,
                focus: Duration::ZERO,
                scroll: Duration::ZERO,
                progress: Duration::ZERO,
                enter: Duration::ZERO,
                exit: Duration::ZERO,
                enter_offset: 0.0,
                expand: Duration::ZERO,
                easing: Easing::EaseOut,
                press_easing: Easing::EaseOut,
            },
        }
    }

    /// Nocturne glass: colorless glass on a deep ink-blue night scene (`NightScene`).
    /// The glass does not tint. Its white fill is at most 15%, and the scene behind it is
    /// blurred by 40 logical pixels with saturation x1.8.
    /// Panels have three layers: outer depth shadow, top highlight and bottom dark edge.
    /// The only accent is champagne gold `#E4B863`. The highlight is moonlight blue
    /// `#7C9CC4`. Transitions take 500 ms, presses 150 ms.
    pub fn nocturne() -> Self {
        let white = |a: f32| Color::WHITE.with_opacity(a);
        let gold = Color::hex(0xE4B863);
        let gold_light = Color::hex(0xF3DCA8);
        let moon = Color::hex(0x7C9CC4);
        let ink = |a: f32| Color::rgbaf(3, 7, 18, a);
        let glass_button = ButtonStyle {
            fill: white(0.12),
            fill_hover: white(0.15),
            fill_pressed: white(0.15),
            border: white(0.40),
            border_hover: white(0.60),
            text: Color::WHITE,
            weight: 500,
            top_highlight: white(0.20),
            glow: Shadow::NONE,
            glow_hover: Shadow::NONE,
            lift: 2.0,
            press_scale: 0.97,
            padding: Insets::symmetric(18.0, 8.0),
            icon_size: 16.0,
            icon_gap: 8.0,
            disabled_fill: white(0.06),
            disabled_text: white(0.35),
            disabled_border: white(0.15),
        };
        Theme {
            background: Color::hex(0x0B1322),
            scene: Some(Rc::new(NightScene::default())),
            backdrop: BackdropStyle { blur: 40.0, saturation: 1.8, downsample: 4 },
            noise: 0.025,
            typography: Typography {
                title: TypeStyle { size: 22.0, weight: 600, color: Color::WHITE },
                body: TypeStyle { size: 14.0, weight: 400, color: white(0.80) },
                secondary: TypeStyle { size: 14.0, weight: 400, color: white(0.60) },
                caption: TypeStyle { size: 12.0, weight: 400, color: white(0.45) },
                line_height: 1.45,
            },
            accent: gold,
            highlight: moon,
            radius: Radii { panel: 14.0, bar: 18.0, control: 10.0, tag: 8.0 },
            spacing: Spacing { panel_padding: 16.0, gap: 10.0 },
            focus_ring: FocusRing { color: gold.with_opacity(0.55), width: 1.5, offset: 3.0 },
            panel: PanelStyle {
                backdrop: true,
                fill: white(0.08),
                fill_hover: white(0.11),
                glow: white(0.12),
                top_highlight: white(0.22),
                bottom_edge: Color::rgbaf(2, 6, 16, 0.35),
                border: white(0.15),
                border_hover: white(0.30),
                shadow: Shadow::new(0.0, 16.0, 40.0, ink(0.5)),
                shadow_hover: Shadow::new(0.0, 22.0, 40.0, ink(0.6)),
                lift: 2.0,
            },
            bar: PanelStyle {
                backdrop: true,
                fill: white(0.08),
                fill_hover: white(0.08),
                glow: white(0.10),
                top_highlight: white(0.18),
                bottom_edge: Color::rgbaf(2, 6, 16, 0.30),
                border: Color::TRANSPARENT,
                border_hover: Color::TRANSPARENT,
                shadow: Shadow::new(0.0, 14.0, 36.0, ink(0.5)),
                shadow_hover: Shadow::new(0.0, 14.0, 36.0, ink(0.5)),
                lift: 0.0,
            },
            divider: white(0.10),
            popover: PanelStyle {
                backdrop: true,
                fill: Color::rgbaf(12, 18, 32, 0.82),
                fill_hover: Color::rgbaf(12, 18, 32, 0.82),
                glow: white(0.06),
                top_highlight: white(0.16),
                bottom_edge: Color::rgbaf(2, 6, 16, 0.4),
                border: white(0.12),
                border_hover: white(0.12),
                shadow: Shadow::new(0.0, 18.0, 44.0, ink(0.6)),
                shadow_hover: Shadow::new(0.0, 18.0, 44.0, ink(0.6)),
                lift: 0.0,
            },
            button: glass_button,
            accent_button: ButtonStyle {
                fill: gold.with_opacity(0.15),
                fill_hover: gold.with_opacity(0.22),
                fill_pressed: gold.with_opacity(0.22),
                border: gold.with_opacity(0.40),
                border_hover: gold.with_opacity(0.55),
                text: gold_light,
                top_highlight: gold_light.with_opacity(0.35),
                glow: Shadow::new(0.0, 0.0, 18.0, gold.with_opacity(0.15)),
                glow_hover: Shadow::new(0.0, 0.0, 24.0, gold.with_opacity(0.22)),
                ..glass_button
            },
            icon_button: IconButtonStyle {
                size: 32.0,
                fill: white(0.10),
                fill_hover: white(0.14),
                border: white(0.15),
                border_hover: white(0.30),
                top_highlight: white(0.20),
                icon: moon,
                icon_size: 16.0,
                press_scale: 0.97,
            },
            input: InputStyle {
                fill: white(0.08),
                fill_hover: white(0.10),
                fill_focus: white(0.12),
                border: white(0.15),
                border_hover: white(0.30),
                border_focus: white(0.50),
                border_width: 0.0,
                border_focus_width: 0.0,
                focus_glow: Shadow::new(0.0, 0.0, 0.0, gold.with_opacity(0.18)).spread(3.0),
                text: Color::WHITE,
                placeholder: white(0.40),
                icon: white(0.50),
                icon_size: 16.0,
                icon_gap: 10.0,
                caret: Color::WHITE,
                selection: moon.with_opacity(0.35),
                preedit: moon,
                padding: Insets::symmetric(12.0, 8.0),
            },
            scroll: ScrollStyle {
                bar_width: 6.0,
                bar_width_hover: 8.0,
                bar_inset: 3.0,
                min_thumb: 32.0,
                thumb: white(0.16),
                thumb_hover: white(0.28),
                thumb_active: white(0.42),
                wheel_step: 56.0,
            },
            slider: SliderStyle {
                track_height: 6.0,
                track: white(0.10),
                track_border: white(0.15),
                fill: moon,
                knob_size: 16.0,
                knob: Color::rgb(236, 240, 246),
                knob_border: white(0.60),
                knob_shadow: Shadow::new(0.0, 2.0, 6.0, ink(0.45)),
                knob_ring: gold.with_opacity(0.85),
                knob_ring_width: 1.5,
                knob_ring_gap: 2.0,
            },
            caption: CaptionStyle {
                width: 46.0,
                height: 34.0,
                icon: white(0.75),
                icon_hover: Color::WHITE,
                icon_size: 10.0,
                fill_hover: white(0.10),
                fill_pressed: white(0.06),
                close_fill_hover: Color::rgbaf(196, 43, 28, 0.90),
                close_fill_pressed: Color::rgbaf(196, 43, 28, 0.70),
                close_icon_hover: Color::WHITE,
                radius: 8.0,
            },
            checkbox: CheckboxStyle {
                size: 18.0,
                radius: 5.0,
                fill: white(0.06),
                fill_hover: white(0.10),
                border: white(0.30),
                border_hover: white(0.50),
                checked_fill: gold,
                checked_fill_hover: gold_light,
                check: Color::hex(0x1A1408),
                label_gap: 10.0,
            },
            tab: TabStyle {
                fill: Color::TRANSPARENT,
                fill_hover: white(0.06),
                fill_selected: white(0.12),
                border_selected: white(0.14),
                text: white(0.60),
                text_hover: white(0.85),
                text_selected: Color::WHITE,
                weight: 500,
                icon_size: 16.0,
                gap: 7.0,
                padding: Insets::symmetric(14.0, 7.0),
                badge_text: white(0.55),
                badge_fill: white(0.08),
                badge_text_selected: Color::hex(0x1A1408),
                badge_fill_selected: gold,
            },
            progress: ProgressStyle {
                height: 6.0,
                track: white(0.10),
                fill: gold,
                fill_end: gold_light,
                indeterminate_width: 0.3,
                indeterminate_period: Duration::from_millis(1600),
                spinner_size: 16.0,
                spinner: white(0.85),
                spinner_period: Duration::from_millis(900),
            },
            list_item: ListItemStyle {
                fill: Color::TRANSPARENT,
                fill_hover: white(0.06),
                fill_pressed: white(0.04),
                fill_selected: white(0.10),
                border_selected: white(0.12),
                padding: Insets::symmetric(12.0, 9.0),
                gap: 12.0,
            },
            menu: MenuStyle {
                radius: 12.0,
                inset: 5.0,
                item_padding: Insets::symmetric(10.0, 6.0),
                icon_size: 16.0,
                gap: 10.0,
                text: white(0.90),
                fill_hover: moon,
                text_hover: Color::WHITE,
                danger: Color::hex(0xF07068),
            },
            tag: TagStyle {
                fill: white(0.08),
                text: white(0.70),
                accent_fill: gold.with_opacity(0.16),
                accent_text: gold_light,
                font_size: 12.0,
                padding: Insets::symmetric(8.0, 1.0),
            },
            disabled_opacity: 0.4,
            motion: Motion {
                hover: Duration::from_millis(500),
                press: Duration::from_millis(150),
                focus: Duration::from_millis(500),
                scroll: Duration::from_millis(220),
                progress: Duration::from_millis(350),
                enter: Duration::from_millis(420),
                exit: Duration::from_millis(180),
                enter_offset: 14.0,
                expand: Duration::from_millis(240),
                easing: Easing::SMOOTH,
                press_easing: Easing::EaseOut,
            },
        }
    }

    pub fn with_scene(mut self, scene: impl Scene) -> Self {
        self.scene = Some(Rc::new(scene));
        self
    }
}
