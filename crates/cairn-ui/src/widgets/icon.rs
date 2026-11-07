//! Built-in icons in the SF Symbols style, drawn on a 24 x 24 grid with artwork inside 2 to 22.
//! One stroke weight with round caps and joins. Only small state or action shapes are solid.
//! Paths are SVG path data, parsed and cached on first use.

use std::sync::OnceLock;

use crate::{Affine, Brush, Color, FillRule, LineCap, LineJoin, Painter, Path, Point, Rect, Seg, StrokeStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Dot,
    Plus,
    Minus,
    Close,
    Check,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    Play,
    Menu,
    Search,
    Cube,
    Image,
    Sun,
    Globe,
    Camera,
    Trash,
    Download,
    Folder,
    Home,
    Gamepad,
    Package,
    Gear,
    Rocket,
    User,
    Refresh,
    Copy,
    ExternalLink,
    Info,
    Warning,
    Stop,
    More,
    Chip,
    Cup,
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
    Puzzle,
    Widgets,
    Terminal,
    Doc,
    Shield,
    ShieldAlert,
    Sliders,
    Stack,
    Bell,
    Braces,
}

impl Icon {
    /// All icons, in enum declaration order. `parts` indexes by discriminant, so keep them in sync.
    pub const ALL: [Icon; 48] = [
        Icon::Dot,
        Icon::Plus,
        Icon::Minus,
        Icon::Close,
        Icon::Check,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::ChevronUp,
        Icon::ChevronDown,
        Icon::Play,
        Icon::Menu,
        Icon::Search,
        Icon::Cube,
        Icon::Image,
        Icon::Sun,
        Icon::Globe,
        Icon::Camera,
        Icon::Trash,
        Icon::Download,
        Icon::Folder,
        Icon::Home,
        Icon::Gamepad,
        Icon::Package,
        Icon::Gear,
        Icon::Rocket,
        Icon::User,
        Icon::Refresh,
        Icon::Copy,
        Icon::ExternalLink,
        Icon::Info,
        Icon::Warning,
        Icon::Stop,
        Icon::More,
        Icon::Chip,
        Icon::Cup,
        Icon::WindowMinimize,
        Icon::WindowMaximize,
        Icon::WindowRestore,
        Icon::Puzzle,
        Icon::Widgets,
        Icon::Terminal,
        Icon::Doc,
        Icon::Shield,
        Icon::ShieldAlert,
        Icon::Sliders,
        Icon::Stack,
        Icon::Bell,
        Icon::Braces,
    ];

    /// English name, used as the default accessibility name.
    pub fn name(self) -> &'static str {
        match self {
            Icon::Dot => "dot",
            Icon::Plus => "add",
            Icon::Minus => "remove",
            Icon::Close => "close",
            Icon::Check => "check",
            Icon::ChevronLeft => "back",
            Icon::ChevronRight => "forward",
            Icon::ChevronUp => "up",
            Icon::ChevronDown => "down",
            Icon::Play => "play",
            Icon::Menu => "menu",
            Icon::Search => "search",
            Icon::Cube => "cube",
            Icon::Image => "image",
            Icon::Sun => "sun",
            Icon::Globe => "globe",
            Icon::Camera => "camera",
            Icon::Trash => "delete",
            Icon::Download => "download",
            Icon::Folder => "folder",
            Icon::Home => "home",
            Icon::Gamepad => "gamepad",
            Icon::Package => "package",
            Icon::Gear => "settings",
            Icon::Rocket => "launch",
            Icon::User => "account",
            Icon::Refresh => "refresh",
            Icon::Copy => "copy",
            Icon::ExternalLink => "open externally",
            Icon::Info => "info",
            Icon::Warning => "warning",
            Icon::Stop => "stop",
            Icon::More => "more",
            Icon::Chip => "memory",
            Icon::Cup => "cup",
            Icon::WindowMinimize => "minimize",
            Icon::WindowMaximize => "maximize",
            Icon::WindowRestore => "restore",
            Icon::Puzzle => "plugin",
            Icon::Widgets => "widgets",
            Icon::Terminal => "terminal",
            Icon::Doc => "document",
            Icon::Shield => "permissions",
            Icon::ShieldAlert => "high risk",
            Icon::Sliders => "adjust",
            Icon::Stack => "instances",
            Icon::Bell => "events",
            Icon::Braces => "code",
        }
    }
}

/// Stroke width in design grid units, close to the SF Symbols Regular weight.
const WEIGHT: f32 = 1.8;
/// Stroke width of the window buttons. Thinner than other icons, to match the system buttons.
const THIN: f32 = 1.2;
/// Minimum stroke width in logical pixels. Small icons would otherwise scale to under 1 pixel,
/// which looks blurry and thinner than the bold text beside them.
const MIN_WIDTH: f32 = 1.4;

enum Stroke {
    Line,
    Solid,
    Thin,
}

pub fn paint_icon(p: &mut dyn Painter, icon: Icon, rect: Rect, color: Color) {
    let d = rect.width.min(rect.height);
    let k = d / 24.0;
    let c = rect.center();
    let at = Affine { scale: k, dx: c.x - 12.0 * k, dy: c.y - 12.0 * k };
    let brush = Brush::Solid(color);
    for (kind, path) in &parts()[icon as usize] {
        let weight = match kind {
            Stroke::Solid => {
                p.fill_path(path, at, FillRule::NonZero, &brush);
                continue;
            }
            Stroke::Line => WEIGHT,
            Stroke::Thin => THIN,
        };
        let style = StrokeStyle::new(weight.max(MIN_WIDTH / k)).cap(LineCap::Round).join(LineJoin::Round);
        p.stroke_path(path, at, &style, &brush);
    }
}

/// Parsed paths for every icon, built once on first use.
fn parts() -> &'static [Vec<(Stroke, Path)>] {
    static PARTS: OnceLock<Vec<Vec<(Stroke, Path)>>> = OnceLock::new();
    PARTS.get_or_init(|| Icon::ALL.iter().map(|&i| build(i)).collect())
}

fn svg(d: &str) -> Path {
    Path::parse(d).expect("内置图标的路径数据有效")
}

fn build(icon: Icon) -> Vec<(Stroke, Path)> {
    use Stroke::{Line, Solid, Thin};
    let line = |d: &str| (Line, svg(d));
    let solid = |d: &str| (Solid, svg(d));
    let dot = |x: f32, y: f32, r: f32| (Solid, Path::circle(Point::new(x, y), r));
    match icon {
        Icon::Dot => vec![dot(12.0, 12.0, 4.0)],
        Icon::Plus => vec![line("M12 5v14M5 12h14")],
        Icon::Minus => vec![line("M5 12h14")],
        Icon::Close => vec![line("M6.5 6.5l11 11M17.5 6.5l-11 11")],
        Icon::Check => vec![line("M5 12.75l4.5 4.5L19 6.75")],
        Icon::ChevronLeft => vec![line("M14.5 5L7.5 12l7 7")],
        Icon::ChevronRight => vec![line("M9.5 5l7 7-7 7")],
        Icon::ChevronUp => vec![line("M5 14.5l7-7 7 7")],
        Icon::ChevronDown => vec![line("M5 9.5l7 7 7-7")],
        // Corners are rounded. The triangle's centroid sits left of its box, so the shape is nudged
        // right to look centered.
        Icon::Play => vec![solid("M9.1 4.9C8.1 4.3 7 5 7 6.1v11.8c0 1.1 1.1 1.8 2.1 1.2l9.6-5.9c.9-.6.9-1.8 0-2.4z")],
        Icon::Menu => vec![line("M4.5 6.5h15M4.5 12h15M4.5 17.5h15")],
        Icon::Search => vec![line("M10.5 4a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM15.3 15.3l4.7 4.7")],
        Icon::Cube => vec![line("M12 2.75l8 4.5v9.5l-8 4.5-8-4.5v-9.5zM4.3 7.4l7.7 4.35 7.7-4.35M12 11.75v9.25")],
        Icon::Image => vec![
            line("M6 4.5h12a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3v-9a3 3 0 0 1 3-3z"),
            line("M3.5 16.6l4.4-4.4a1.6 1.6 0 0 1 2.2 0l5.3 5.3M14 15.2l1.5-1.5a1.6 1.6 0 0 1 2.2 0l3 3"),
            dot(15.75, 8.75, 1.65),
        ],
        Icon::Sun => {
            let mut rays = Path::new();
            for i in 0..8 {
                let (s, c) = (i as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
                rays.move_to(12.0 + c * 7.4, 12.0 + s * 7.4).line_to(12.0 + c * 9.4, 12.0 + s * 9.4);
            }
            vec![line("M12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8z"), (Line, rays)]
        }
        Icon::Globe => vec![line(
            "M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 3c-2.4 2.3-3.75 5.5-3.75 9s1.35 6.7 3.75 9M12 3c2.4 2.3 3.75 5.5 3.75 9s-1.35 6.7-3.75 9M3.3 9.25h17.4M3.3 14.75h17.4",
        )],
        Icon::Camera => vec![
            line(
                "M5.75 7H8l1.15-1.9a1.8 1.8 0 0 1 1.55-.9h2.6a1.8 1.8 0 0 1 1.55.9L16 7h2.25a3 3 0 0 1 3 3v7.5a3 3 0 0 1-3 3H5.75a3 3 0 0 1-3-3V10a3 3 0 0 1 3-3z",
            ),
            line("M12 10a3.5 3.5 0 1 1 0 7 3.5 3.5 0 0 1 0-7z"),
        ],
        Icon::Trash => vec![line(
            "M4 6.5h16M9 6.5V5a1.5 1.5 0 0 1 1.5-1.5h3A1.5 1.5 0 0 1 15 5v1.5M6 6.5l.85 12.15a2 2 0 0 0 2 1.85h6.3a2 2 0 0 0 2-1.85L18 6.5M10 10.5v6M14 10.5v6",
        )],
        Icon::Download => vec![line("M12 3.5v11M7.5 10l4.5 4.5 4.5-4.5M4 14.5v3A2.5 2.5 0 0 0 6.5 20h11a2.5 2.5 0 0 0 2.5-2.5v-3")],
        Icon::Folder => vec![line(
            "M3 7.5A2.5 2.5 0 0 1 5.5 5h3.55a2 2 0 0 1 1.5.68l1.35 1.57h6.6A2.5 2.5 0 0 1 21 9.75v7.75a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5zM3 10h18",
        )],
        Icon::Home => vec![line("M3 11.25l9-7.5 9 7.5M5.5 9.25v9.25a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V9.25M10 20.5v-5a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v5")],
        Icon::Gamepad => vec![
            line(
                "M7.4 6.5h9.2a4.5 4.5 0 0 1 4.4 3.6l1 5.1a3.1 3.1 0 0 1-5.35 2.7L15 16H9l-1.65 1.9A3.1 3.1 0 0 1 2 15.2l1-5.1a4.5 4.5 0 0 1 4.4-3.6zM7.75 9.75v3.5M6 11.5h3.5",
            ),
            dot(15.6, 10.2, 1.05),
            dot(17.6, 12.4, 1.05),
        ],
        Icon::Package => vec![line(
            "M4 4.5h16a1.5 1.5 0 0 1 1.5 1.5v1.5A1.5 1.5 0 0 1 20 9H4a1.5 1.5 0 0 1-1.5-1.5V6A1.5 1.5 0 0 1 4 4.5zM4.25 9v8.5a2.5 2.5 0 0 0 2.5 2.5h10.5a2.5 2.5 0 0 0 2.5-2.5V9M10 12.75h4",
        )],
        Icon::Gear => vec![(Line, gear()), line("M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6z")],
        // Drawn upright first, then rotated 45 degrees to fly toward the upper right.
        Icon::Rocket => {
            let body = svg(
                "M12 2.75c2.6 1.9 4 4.9 4 8.4v4.6a1.25 1.25 0 0 1-1.25 1.25h-5.5A1.25 1.25 0 0 1 8 15.75v-4.6c0-3.5 1.4-6.5 4-8.4zM8 12.75l-2.4 2a1.5 1.5 0 0 0-.55 1.15v3.35L8 17.25M16 12.75l2.4 2a1.5 1.5 0 0 1 .55 1.15v3.35L16 17.25M10.5 19.75v1M13.5 19.75v1",
            );
            let window = Path::circle(Point::new(12.0, 9.75), 1.6);
            vec![(Line, rotated(&body, 45.0)), (Solid, rotated(&window, 45.0))]
        }
        Icon::User => vec![line(
            "M12 4a3.75 3.75 0 1 1 0 7.5A3.75 3.75 0 0 1 12 4zM4.75 20c.3-3.6 3.4-6.1 7.25-6.1s6.95 2.5 7.25 6.1a.8.8 0 0 1-.8.9H5.55a.8.8 0 0 1-.8-.9z",
        )],
        // Arc starts at the upper right, goes clockwise almost a full turn and ends straight above.
        // The arrow head at the end points right.
        Icon::Refresh => vec![line("M17.66 6.84A8 8 0 1 1 12 4.5M9.5 1.9L12 4.5l-2.5 2.6")],
        Icon::Copy => vec![line(
            "M10.5 8h8a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1-2.5 2.5h-8A2.5 2.5 0 0 1 8 18.5v-8A2.5 2.5 0 0 1 10.5 8zM16 8V5.5A2.5 2.5 0 0 0 13.5 3h-8A2.5 2.5 0 0 0 3 5.5v8A2.5 2.5 0 0 0 5.5 16H8",
        )],
        Icon::ExternalLink => vec![line("M11 4.5H6.5A2.5 2.5 0 0 0 4 7v10.5A2.5 2.5 0 0 0 6.5 20H17a2.5 2.5 0 0 0 2.5-2.5V13M14 4h6v6M20 4l-8.5 8.5")],
        Icon::Info => vec![line("M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 11v5.75M10.75 11H12"), dot(12.0, 7.75, 1.2)],
        Icon::Warning => vec![line("M10.27 4.5a2 2 0 0 1 3.46 0l7.08 12.5A2 2 0 0 1 19.08 20H4.92a2 2 0 0 1-1.73-3zM12 9.25v4.5"), dot(12.0, 16.6, 1.15)],
        Icon::Stop => vec![solid("M9 6h6a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H9a3 3 0 0 1-3-3V9a3 3 0 0 1 3-3z")],
        Icon::More => vec![dot(5.75, 12.0, 1.65), dot(12.0, 12.0, 1.65), dot(18.25, 12.0, 1.65)],
        Icon::Chip => vec![line(
            "M6 6.5h12a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM8 6.5V3.75M12 6.5V3.75M16 6.5V3.75M8 17.5v2.75M12 17.5v2.75M16 17.5v2.75M9 10h6v4H9z",
        )],
        Icon::Cup => vec![line("M5 7.5h11v5a5 5 0 0 1-5 5h-1a5 5 0 0 1-5-5zM16 9h1.25a2.5 2.5 0 0 1 0 5H16M3.5 20.5h15M9 2.75v2M12 2.75v2")],
        Icon::WindowMinimize => vec![(Thin, svg("M1 12h22"))],
        Icon::WindowMaximize => vec![(Thin, svg("M4 1h16a3 3 0 0 1 3 3v16a3 3 0 0 1-3 3H4a3 3 0 0 1-3-3V4a3 3 0 0 1 3-3z"))],
        Icon::WindowRestore => vec![(
            Thin,
            svg("M3.5 6h13A2.5 2.5 0 0 1 19 8.5v13a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 1 21.5v-13A2.5 2.5 0 0 1 3.5 6zM6 1h14.5A2.5 2.5 0 0 1 23 3.5V18"),
        )],
        // A rounded square with round tabs on the top and right edges. Tabs are about half the
        // body's width, so the shape reads as a puzzle piece rather than a mug at small sizes.
        Icon::Puzzle => {
            vec![line("M5.9 9.1h1.8A2.9 2.9 0 1 1 11.1 9.1h1.8a2 2 0 0 1 2 2v1.8A2.9 2.9 0 1 1 14.9 16.3v1.8a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z")]
        }
        Icon::Widgets => {
            let tile = |x: f32, y: f32| line(&format!("M{} {y}h3.5a2 2 0 0 1 2 2v3.5a2 2 0 0 1-2 2h-3.5a2 2 0 0 1-2-2v-3.5a2 2 0 0 1 2-2z", x + 2.0));
            vec![tile(3.5, 3.5), tile(13.0, 3.5), tile(3.5, 13.0), tile(13.0, 13.0)]
        }
        Icon::Terminal => vec![line("M6 4.5h12a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3v-9a3 3 0 0 1 3-3zM7.5 9.25l3 2.75-3 2.75M12.75 15h4")],
        Icon::Doc => vec![line(
            "M7 3h6.6a2 2 0 0 1 1.4.6l3.4 3.4a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM13.5 3.25V7a1.5 1.5 0 0 0 1.5 1.5h3.75M8.5 12.5h7M8.5 16h5",
        )],
        Icon::Shield => vec![(Line, shield()), line("M8.75 11.75l2.25 2.25 4.25-4.5")],
        Icon::ShieldAlert => vec![(Line, shield()), line("M12 7.75v4.5"), dot(12.0, 15.4, 1.15)],
        // Three tracks, each with a knob at a different position.
        Icon::Sliders => {
            let knob = |x: f32, y: f32| line(&format!("M{x} {}a2 2 0 1 1 0 4 2 2 0 0 1 0-4z", y - 2.0));
            vec![line("M3.5 6.5h10M17.5 6.5h3M3.5 12h3M10.5 12h10M3.5 17.5h8.5M16 17.5h4.5"), knob(15.5, 6.5), knob(8.5, 12.0), knob(14.0, 17.5)]
        }
        Icon::Stack => vec![line("M12 3.5l8.5 4.25L12 12 3.5 7.75zM3.5 12l8.5 4.25L20.5 12M3.5 16.25l8.5 4.25 8.5-4.25")],
        // The flare at the rim mirrors exactly on both sides.
        Icon::Bell => vec![line(
            "M12 2.9a5.75 5.75 0 0 1 5.75 5.75v3.6c0 1 .35 1.95 1 2.7l.7.8c.55.65.1 1.65-.75 1.65H5.3c-.85 0-1.3-1-.75-1.65l.7-.8c.65-.75 1-1.7 1-2.7v-3.6A5.75 5.75 0 0 1 12 2.9zM10 19.25a2 2 0 0 0 4 0",
        )],
        Icon::Braces => vec![line(
            "M8.5 4.5h-.75a2 2 0 0 0-2 2v3.25A2.25 2.25 0 0 1 3.5 12a2.25 2.25 0 0 1 2.25 2.25v3.25a2 2 0 0 0 2 2h.75M15.5 4.5h.75a2 2 0 0 1 2 2v3.25A2.25 2.25 0 0 0 20.5 12a2.25 2.25 0 0 0-2.25 2.25v3.25a2 2 0 0 1-2 2h-.75",
        )],
    }
}

/// Shield outline shared by the permission icons: a softly rounded top and a pointed base.
fn shield() -> Path {
    svg(
        "M11.3 3.05a1.9 1.9 0 0 1 1.4 0l5.4 2.15a1.75 1.75 0 0 1 1.15 1.65v4.4c0 4.55-2.95 8.25-7.25 10-4.3-1.75-7.25-5.45-7.25-10v-4.4a1.75 1.75 0 0 1 1.15-1.65z",
    )
}

/// Gear outline: 8 teeth. Tooth tips and roots are concentric arcs joined into one closed path.
/// Teeth are narrower at the top than at the base.
fn gear() -> Path {
    const TEETH: usize = 8;
    let (outer, root) = (9.6f32, 7.3f32);
    let step = std::f32::consts::TAU / TEETH as f32;
    let (top, base) = (step * 0.2, step * 0.3);
    let at = |r: f32, a: f32| (12.0 + r * a.cos(), 12.0 + r * a.sin());
    let mut d = String::new();
    for i in 0..TEETH {
        let a = i as f32 * step - std::f32::consts::FRAC_PI_2;
        let (x0, y0) = at(root, a - base);
        let (x1, y1) = at(outer, a - top);
        let (x2, y2) = at(outer, a + top);
        let (x3, y3) = at(root, a + base);
        let (x4, y4) = at(root, a + step - base);
        if i == 0 {
            d += &format!("M{x0} {y0}");
        }
        d += &format!("L{x1} {y1}A{outer} {outer} 0 0 1 {x2} {y2}L{x3} {y3}A{root} {root} 0 0 1 {x4} {y4}");
    }
    d.push('Z');
    svg(&d)
}

/// Rotates the path clockwise by `deg` degrees around the center of the design grid.
fn rotated(path: &Path, deg: f32) -> Path {
    let (s, c) = deg.to_radians().sin_cos();
    let r = |p: &Point| {
        let (x, y) = (p.x - 12.0, p.y - 12.0);
        Point::new(12.0 + x * c - y * s, 12.0 + x * s + y * c)
    };
    let mut out = Path::new();
    for seg in path.segments() {
        match seg {
            Seg::Move(a) => {
                let a = r(a);
                out.move_to(a.x, a.y);
            }
            Seg::Line(a) => {
                let a = r(a);
                out.line_to(a.x, a.y);
            }
            Seg::Quad(a, b) => {
                let (a, b) = (r(a), r(b));
                out.quad_to(a.x, a.y, b.x, b.y);
            }
            Seg::Cubic(a, b, e) => {
                let (a, b, e) = (r(a), r(b), r(e));
                out.cubic_to(a.x, a.y, b.x, b.y, e.x, e.y);
            }
            Seg::Close => {
                out.close();
            }
        }
    }
    out
}
