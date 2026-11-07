//! Rendering layer of cairn-ui. `Painter` is the backend-neutral drawing API.
//! `SkiaPainter` and `Canvas` are the tiny-skia CPU backend; `TextSystem` handles fonts.
//! Coordinates are logical pixels, except `DeviceRect` and `Canvas`, which use physical pixels.

pub mod corner;
pub mod effects;
mod geom;
mod image;
mod painter;
mod path;
mod scene;
#[cfg(target_arch = "x86_64")]
mod simd;
mod skia;
mod text;

pub use geom::{Affine, Color, DeviceRect, Point, Rect, Size};
pub use image::Image;
pub use painter::{Brush, FillRule, GradientStop, LineCap, LineJoin, Painter, Shadow, StrokeStyle};
pub use path::{Path, PathParseError, Seg};
pub use scene::{ImageScene, NightScene, Scene, Spot};
pub use skia::{BackdropStyle, Background, Canvas, SceneStatus, SkiaPainter};
pub use text::{FontFamily, FontSetup, TextLayout, TextStyle, TextSystem};
