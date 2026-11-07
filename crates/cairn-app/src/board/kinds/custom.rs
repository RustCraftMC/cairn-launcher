//! Custom widgets: each `<data dir>/widgets/*.yaml` file is one kind (`custom:<id>`), and each widget
//! a plugin declares is another (`plugin:<plugin>/<widget>`). Both are drawn by the custom panel.
//! Kinds are built at run time and leaked to `'static` (the widget lists need `&'static Kind`),
//! reused while the definition is unchanged. [`super::find`] and [`super::all`] include them.

use cairn_i18n::tr;
use std::cell::RefCell;

use cairn_bridge::{Blocked, CustomWidgetDef, ExtensionKind, Model};
use cairn_ui::{El, Icon, Styled, button, column, label, scroll};

use super::{Category, Ctx, Kind, Opt, Preset, card, heading, span};
use crate::board::glyph::Glyph;
use crate::board::grid::{COLS, ROWS};
use crate::gui::Gui;
use crate::style::accent;
use crate::views::custom_panel::{PluginScope, Scope, Surface, render};

/// Prefix of YAML widget kind ids as stored in layouts.
pub const PREFIX: &str = "custom:";
/// Prefix of plugin widget kind ids as stored in layouts.
pub const PLUGIN_PREFIX: &str = "plugin:";

/// Fields that shape the generated `Kind`. A new `Kind` is made only when they change.
/// `view` looks up the definition again on each paint.
#[derive(Clone, PartialEq)]
struct Meta {
    kind: String,
    name: String,
    summary: String,
    icon: Option<String>,
    size: (u16, u16),
    sizes: Vec<(u16, u16)>,
    min: (u16, u16),
    max: (u16, u16),
}

thread_local! {
    /// Cached kinds survive disable/enable cycles; only active entries appear in the gallery.
    static REGISTRY: RefCell<Vec<(Meta, &'static Kind, bool)>> = const { RefCell::new(Vec::new()) };
}

const OPTIONS: &[Opt] = &[Opt::Action { label: "Reload YAML", icon: Icon::Refresh, run: reload }];

fn reload(g: &mut Gui) {
    g.bridge.reload_panels(&mut g.m);
    g.bridge.reload_plugins(&mut g.m, false);
}

fn is_custom(kind: &str) -> bool {
    kind.starts_with(PREFIX) || kind.starts_with(PLUGIN_PREFIX)
}

/// Syncs the registry with the YAML and plugin widgets loaded into the model. Unchanged definitions
/// only cost a few comparisons, so this can run on every UI rebuild.
pub fn sync(m: &Model) {
    let items = m.board.as_ref().map_or(&[][..], |layout| layout.items.as_slice());
    let yaml = m.widgets.iter().map(|def| (format!("{PREFIX}{}", def.id), def));
    let plugins = m.plugins.widgets.iter().map(|w| (w.kind.clone(), &w.def));
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        for (_, _, available) in r.iter_mut() {
            *available = false;
        }
        for (kind, def) in yaml.chain(plugins) {
            let m = meta(kind, def);
            if let Some((_, _, available)) = r.iter_mut().find(|(old, _, _)| old == &m) {
                *available = true;
            } else {
                let kind = leak(&m);
                r.push((m, kind, true));
            }
        }
        for item in items {
            if is_custom(&item.kind) && !r.iter().any(|(m, _, _)| m.kind == item.kind) {
                let name = item.kind.strip_prefix(PREFIX).or_else(|| item.kind.strip_prefix(PLUGIN_PREFIX)).unwrap_or(&item.kind);
                let m = Meta {
                    kind: item.kind.clone(),
                    name: name.into(),
                    summary: String::new(),
                    icon: None,
                    size: (item.w, item.h),
                    sizes: Vec::new(),
                    min: (1, 1),
                    max: (COLS, ROWS),
                };
                let kind = leak(&m);
                r.push((m, kind, false));
            }
        }
    });
}

pub fn all() -> Vec<&'static Kind> {
    REGISTRY.with(|r| r.borrow().iter().filter(|(_, _, available)| *available).map(|(_, k, _)| *k).collect())
}

pub fn find(id: &str) -> Option<&'static Kind> {
    REGISTRY.with(|r| {
        let r = r.borrow();
        r.iter().rev().find(|(_, k, active)| k.id == id && *active).or_else(|| r.iter().rev().find(|(_, k, _)| k.id == id)).map(|(_, k, _)| *k)
    })
}

/// Whether a widget of this kind can be added now: its YAML file or plugin is loaded and enabled.
pub fn available(m: &Model, kind: &str) -> bool {
    match kind.strip_prefix(PREFIX) {
        Some(id) => m.widgets.iter().any(|def| def.id == id),
        None => m.plugins.widget(kind).is_some(),
    }
}

fn meta(kind: String, d: &CustomWidgetDef) -> Meta {
    Meta {
        kind,
        name: d.name.clone(),
        summary: d.summary.clone().unwrap_or_else(|| tr!("board.custom_title").into()),
        icon: d.icon.clone(),
        size: d.size,
        sizes: d.sizes.clone(),
        min: d.min,
        max: d.max,
    }
}

fn leak(m: &Meta) -> &'static Kind {
    let text = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
    let min = span(m.min.0.clamp(1, COLS), m.min.1.clamp(1, ROWS));
    let max = span(m.max.0.clamp(min.w, COLS), m.max.1.clamp(min.h, ROWS));
    let fit = |(w, h): (u16, u16)| span(w.clamp(min.w, max.w), h.clamp(min.h, max.h));
    let mut presets = vec![Preset { label: "Default", span: fit(m.size) }];
    for &s in &m.sizes {
        let s = fit(s);
        if presets.iter().all(|p| p.span != s) {
            presets.push(Preset { label: text(&format!("{}×{}", s.w, s.h)), span: s });
        }
    }
    Box::leak(Box::new(Kind {
        id: text(&m.kind),
        name: text(&m.name),
        summary: text(&m.summary),
        glyph: glyph_for(m.icon.as_deref()),
        category: Category::Custom,
        min,
        max,
        presets: Box::leak(presets.into_boxed_slice()),
        default: 0,
        render: view,
        options: OPTIONS,
    }))
}

/// Maps the YAML `icon` name to a glyph. Unknown names get a sparkle.
pub fn glyph_for(name: Option<&str>) -> Glyph {
    match name.unwrap_or("") {
        "calendar" => Glyph::Calendar,
        "flame" | "fire" => Glyph::Flame,
        "chart" => Glyph::Chart,
        "clock" => Glyph::Clock,
        "quote" => Glyph::Quote,
        "bolt" => Glyph::Bolt,
        "pulse" => Glyph::Pulse,
        "grid" => Glyph::Grid,
        "globe" | "server" => Glyph::Icon(Icon::Globe),
        "rocket" => Glyph::Icon(Icon::Rocket),
        "play" => Glyph::Icon(Icon::Play),
        "folder" => Glyph::Icon(Icon::Folder),
        "info" => Glyph::Icon(Icon::Info),
        "bell" | "events" => Glyph::Icon(Icon::Bell),
        "puzzle" | "plugin" => Glyph::Icon(Icon::Puzzle),
        "widgets" | "apps" => Glyph::Icon(Icon::Widgets),
        "terminal" | "console" => Glyph::Icon(Icon::Terminal),
        "doc" | "file" | "text" => Glyph::Icon(Icon::Doc),
        "shield" => Glyph::Icon(Icon::Shield),
        "sliders" | "tune" => Glyph::Icon(Icon::Sliders),
        "stack" | "layers" => Glyph::Icon(Icon::Stack),
        "code" | "braces" => Glyph::Icon(Icon::Braces),
        "package" | "box" => Glyph::Icon(Icon::Package),
        "download" => Glyph::Icon(Icon::Download),
        "gamepad" => Glyph::Icon(Icon::Gamepad),
        "user" => Glyph::Icon(Icon::User),
        "home" => Glyph::Icon(Icon::Home),
        "gear" | "settings" => Glyph::Icon(Icon::Gear),
        "image" => Glyph::Icon(Icon::Image),
        "cup" | "coffee" => Glyph::Icon(Icon::Cup),
        "sun" => Glyph::Icon(Icon::Sun),
        _ => Glyph::Sparkle,
    }
}

/// Shown in place of a widget whose YAML file or plugin is missing, broken or disabled. The layout
/// item and its options stay, so the widget comes back when its source does.
fn placeholder(cx: &Ctx, name: &str, text: &'static str, manage: impl Fn(&mut Gui) + 'static) -> El<Gui> {
    card().child(heading(cx.kind.glyph, name)).child(label(text).caption()).child(button(tr!("extensions.manage")).text_only().on_click(manage)).into()
}

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    if let Some(rest) = cx.kind.id.strip_prefix(PLUGIN_PREFIX) {
        return plugin_view(g, cx, rest);
    }
    let id = cx.kind.id.strip_prefix(PREFIX).unwrap_or(cx.kind.id);
    let Some(def) = g.m.widgets.iter().find(|d| d.id == id) else {
        let entry = g.m.extensions.entries.iter().find(|entry| entry.kind == ExtensionKind::Widget && entry.id.as_deref() == Some(id));
        let text = if g.m.extensions.error.is_some() || entry.is_some_and(|entry| entry.issue.is_some()) {
            tr!("extensions.widget_error")
        } else if entry.is_some() {
            tr!("extensions.widget_disabled")
        } else {
            tr!("board.custom_yaml_missing")
        };
        let id = id.to_string();
        let name = entry.map(|entry| entry.name.as_str()).unwrap_or(cx.kind.name);
        return placeholder(cx, name, text, move |g: &mut Gui| crate::views::extensions::open(g, Some(&id)));
    };
    body(g, cx, def.heading, &def.name, &def.view, Scope { interactive: !cx.preview, plugin: None })
}

fn plugin_view(g: &Gui, cx: &Ctx, rest: &str) -> El<Gui> {
    let (plugin, widget) = rest.split_once('/').unwrap_or((rest, ""));
    let Some(w) = g.m.plugins.widget(cx.kind.id) else {
        let entry = g.m.plugins.entry(plugin);
        let text = match entry.map(|e| g.m.plugins.blocked(e)) {
            Some(Some(Blocked::Invalid)) => tr!("extensions.widget_error"),
            Some(_) => tr!("extensions.widget_disabled"),
            None => tr!("extensions.plugin_missing"),
        };
        let plugin = plugin.to_string();
        let name = entry.and_then(|e| e.manifest.as_ref()).map_or(cx.kind.name, |m| m.name.as_str());
        return placeholder(cx, name, text, move |g: &mut Gui| crate::views::extensions::open_plugin(g, &plugin));
    };
    let live = g.m.plugins.live.get(cx.kind.id);
    let items = live.and_then(|l| l.view.as_deref()).unwrap_or(&w.def.view);
    let scope = PluginScope { plugin, surface: Surface::Widget(widget), data: live.map(|l| &l.data) };
    body(g, cx, w.def.heading, &w.def.name, items, Scope { interactive: !cx.preview, plugin: Some(scope) })
}

fn body(g: &Gui, cx: &Ctx, show_heading: bool, name: &str, items: &[cairn_bridge::PanelItem], scope: Scope) -> El<Gui> {
    let ink = accent();
    let mut c = card();
    if show_heading {
        c = c.child(heading(cx.kind.glyph, name.to_string()));
    }
    let body = column().fill_width().gap(8.0).children(items.iter().map(|it| render(g, it, ink, scope)));
    // Scroll inside the card when the content is taller than the cell, so the widget does not grow.
    c.child(scroll().fill_width().grow(1.0).min_height(0.0).child(body)).into()
}
