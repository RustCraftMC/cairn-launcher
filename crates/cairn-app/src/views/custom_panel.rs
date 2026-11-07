//! Custom panel renderer: builds the element tree declared in YAML at runtime.
//! Supports elements such as Banner, Card, Grid, Row, Column, Text, Button and Progress, and actions
//! such as launching the game, joining a server directly, opening links and copying text.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use cairn_bridge::{PanelAction, PanelItem};
use cairn_i18n::tr;
use cairn_ui::{
    Align, Brush, Change, El, Icon, Image, Insets, Justify, LayoutCx, MeasureInput, PaintCx, Painter, Rect, Size, Style,
    Styled, Widget, button, checkbox, column, divider, icon, label, panel, progress_bar, row, scroll, tab, text_input,
};

use crate::board::glyph::glyph;
use crate::board::kinds::custom::glyph_for;
use crate::gui::{Gui, Sheet};
use crate::style::{accent, fg};
use crate::views::{frame, header, page_reveal, section};

pub fn view(g: &Gui, panel_id: &str) -> El<Gui> {
    let panel_def = match g.m.panels.iter().find(|p| p.id == panel_id) {
        Some(p) => p.clone(),
        None => {
            return frame(
                header(label(tr!("custom_panel.title")).title(), Vec::new()),
                column().grow(1.0).align(Align::Center).justify(Justify::Center).child(label(tr!("custom_panel.not_found", id = panel_id)).secondary()).into(),
                None,
            );
        }
    };

    let accent_color = accent();
    let has_tabs = !panel_def.tabs.is_empty();

    let active_tab_index = 0;
    let current_view_items = if has_tabs { panel_def.tabs.get(active_tab_index).map(|t| &t.view).unwrap_or(&panel_def.view) } else { &panel_def.view };

    let title_lead = row().gap(8.0).align(Align::Center).child(icon(Icon::Globe).icon_size(18.0).color(accent_color)).child(label(&panel_def.title).title());

    let mut actions = Vec::new();
    actions.push(button(tr!("custom_panel.reload")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.reload_panels(&mut g.m)).into());

    let content_children: Vec<El<Gui>> = current_view_items.iter().map(|item| render_item(g, item, accent_color)).collect();

    let body =
        scroll().fill_width().fill_height().padding(Insets::new(14.0, 18.0, 18.0, 18.0)).child(column().fill_width().gap(12.0).children(content_children));

    frame(header(title_lead, actions), page_reveal(panel_id, body), None)
}

/// A page of a plugin: a panel the plugin provides, shown as a sheet that is not in the dock.
/// Its commands, inputs and `open_panel` actions go to the plugin; the view is the latest one the
/// plugin sent with `cairn.panels.setView`, or the declared one until then.
pub fn plugin_view(g: &Gui, plugin: &str, panel: &str) -> El<Gui> {
    let Some(p) = g.m.plugins.panel(plugin, panel) else {
        // The plugin was disabled, broke or was removed while its panel was open.
        let plugin = plugin.to_string();
        let body = column()
            .grow(1.0)
            .align(Align::Center)
            .justify(Justify::Center)
            .gap(10.0)
            .child(label(tr!("custom_panel.plugin_missing")).secondary())
            .child(button(tr!("extensions.manage")).text_only().on_click(move |g: &mut Gui| super::extensions::open_plugin(g, &plugin)));
        return frame(header(label(tr!("custom_panel.title")).title(), Vec::new()), body.into(), None);
    };
    let live = g.m.plugins.live.get(&p.key);
    let items = live.and_then(|l| l.view.as_deref()).unwrap_or(&p.def.view);
    let view_gen = live.map(|l| l.view_gen).unwrap_or(0);
    let scope = Scope { interactive: true, plugin: Some(PluginScope { plugin, surface: Surface::Panel(panel), data: live.map(|l| &l.data) }) };
    let ink = accent();
    let title = row().gap(8.0).align(Align::Center).child(glyph(glyph_for(p.def.icon.as_deref())).size(18.0).color(ink)).child(label(&p.def.title).title());
    let content = column().fill_width().gap(12.0).children(items.iter().map(|item| render(g, item, ink, scope)));
    let body = scroll()
        .key(("plugin-panel-scroll", plugin, panel, view_gen))
        .fill_width()
        .fill_height()
        .padding(Insets::new(14.0, 18.0, 18.0, 18.0))
        .child(content);
    frame(header(title, Vec::new()), page_reveal(("plugin-panel", plugin, panel, view_gen), body), None)
}

/// Where an element is drawn. Plugin widgets and panels add their owner and the data the plugin sent.
#[derive(Clone, Copy)]
pub(crate) struct Scope<'a> {
    pub interactive: bool,
    pub plugin: Option<PluginScope<'a>>,
}

/// The widget or panel of a plugin that an element belongs to.
#[derive(Clone, Copy)]
pub(crate) enum Surface<'a> {
    Widget(&'a str),
    Panel(&'a str),
}

#[derive(Clone, Copy)]
pub(crate) struct PluginScope<'a> {
    pub plugin: &'a str,
    pub surface: Surface<'a>,
    pub data: Option<&'a BTreeMap<String, String>>,
}

/// Who a rendered element reports to once clicked: the plugin and its widget or panel.
#[derive(Clone)]
enum Owner {
    Widget { plugin: String, widget: String },
    Panel { plugin: String, panel: String },
}

impl Owner {
    fn of(scope: PluginScope) -> Self {
        let plugin = scope.plugin.to_string();
        match scope.surface {
            Surface::Widget(widget) => Owner::Widget { plugin, widget: widget.to_string() },
            Surface::Panel(panel) => Owner::Panel { plugin, panel: panel.to_string() },
        }
    }

    fn plugin(&self) -> &str {
        match self {
            Owner::Widget { plugin, .. } | Owner::Panel { plugin, .. } => plugin,
        }
    }

    /// Where text typed into an input waits until it is submitted, in `Plugins::drafts`.
    fn draft(&self, key: &str) -> String {
        match self {
            Owner::Widget { plugin, widget } => format!("{plugin}/{widget}/{key}"),
            Owner::Panel { plugin, panel } => format!("panel:{plugin}/{panel}/{key}"),
        }
    }
}

/// Replaces `{data.key}` with the value a plugin sent for its widget. Values not sent yet show as a dash.
fn fill_data(s: &str, data: Option<&BTreeMap<String, String>>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{data.") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 6..];
        let Some(end) = tail.find('}') else {
            rest = &rest[start..];
            break;
        };
        out.push_str(data.and_then(|d| d.get(&tail[..end])).map_or("–", String::as_str));
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Replaces placeholders in text: `{player}` (current account), `{instance}` (selected instance),
/// `{time}` (hour:minute), `{date}` (month and day), `{weekday}`. Unknown ones stay as they are.
/// The home page rebuilds every minute, so the time stays current. In plugin widgets `{data.key}`
/// is filled first, so plugin data cannot inject the other placeholders.
fn fill(g: &Gui, s: &str, cx: Scope) -> String {
    if !s.contains('{') {
        return s.to_string();
    }
    if let Some(p) = cx.plugin {
        return fill_data(s, p.data);
    }

    let t = cairn_bridge::local_now();
    let player = g.m.selected_account().map_or(tr!("custom_panel.default_player"), |a| a.name.as_str());
    let instance = g.m.selected_instance().map_or(tr!("custom_panel.no_instance"), |i| i.name.as_str());
    s.replace("{player}", player)
        .replace("{instance}", instance)
        .replace("{time}", &format!("{:02}:{:02}", t.hour, t.minute))
        .replace("{date}", &if cairn_i18n::is_chinese() { format!("{}月{}日", t.month, t.day) } else { format!("{}/{}", t.month, t.day) })
        .replace("{weekday}", &crate::board::kinds::weekday_label(t.weekday as usize).to_string())
}

/// The picture URLs that YAML panels and widgets and the widgets and panels of plugins name right now, filled the
/// same way [`render`] fills them. `Gui::after_update` keeps the decoded pictures of these in the model and drops the rest.
pub(crate) fn view_images(g: &Gui, out: &mut HashSet<String>) {
    let m = &g.m;
    let yaml = Scope { interactive: false, plugin: None };
    for panel in &m.panels {
        collect_images(g, &panel.view, yaml, out);
        for tab in &panel.tabs {
            collect_images(g, &tab.view, yaml, out);
        }
    }
    for widget in &m.widgets {
        collect_images(g, &widget.view, yaml, out);
    }
    // A plugin's own view replaces the declared one, so only the view that is drawn counts.
    for w in &m.plugins.widgets {
        let live = m.plugins.live.get(&w.kind);
        let plugin = PluginScope { plugin: &w.plugin, surface: Surface::Widget(&w.def.id), data: live.map(|l| &l.data) };
        collect_images(g, live.and_then(|l| l.view.as_deref()).unwrap_or(&w.def.view), Scope { interactive: false, plugin: Some(plugin) }, out);
    }
    for p in &m.plugins.panels {
        let live = m.plugins.live.get(&p.key);
        let plugin = PluginScope { plugin: &p.plugin, surface: Surface::Panel(&p.def.id), data: live.map(|l| &l.data) };
        collect_images(g, live.and_then(|l| l.view.as_deref()).unwrap_or(&p.def.view), Scope { interactive: false, plugin: Some(plugin) }, out);
    }
}

fn collect_images(g: &Gui, items: &[PanelItem], cx: Scope, out: &mut HashSet<String>) {
    for item in items {
        match item {
            PanelItem::Image { url, .. } => {
                out.insert(fill(g, url, cx));
            }
            PanelItem::Card { children, .. } | PanelItem::Grid { children, .. } | PanelItem::Row { children, .. } | PanelItem::Column { children, .. } => {
                collect_images(g, children, cx, out)
            }
            _ => {}
        }
    }
}

/// Renders one element, shared by custom panels and custom widgets. Text placeholders are filled as in [`fill`].
pub(crate) fn render_item(g: &Gui, item: &PanelItem, accent_color: cairn_ui::Color) -> El<Gui> {
    render(g, item, accent_color, Scope { interactive: true, plugin: None })
}

pub(crate) fn render_preview(g: &Gui, item: &PanelItem, accent_color: cairn_ui::Color) -> El<Gui> {
    render(g, item, accent_color, Scope { interactive: false, plugin: None })
}

pub(crate) fn render(g: &Gui, item: &PanelItem, accent_color: cairn_ui::Color, cx: Scope) -> El<Gui> {
    let owner = cx.plugin.map(Owner::of);
    match item {
        PanelItem::Banner { title, subtitle, height, dim: _, button_text, action, .. } => {
            let h = height.unwrap_or(120.0);
            let action_clone = action.clone();

            let mut banner_col = column()
                .fill_width()
                .height(h)
                .padding(Insets::new(16.0, 20.0, 16.0, 20.0))
                .justify(Justify::Center)
                .gap(6.0)
                .child(label(fill(g, title, cx)).weight(800).color(fg(0.98)));

            if let Some(sub) = subtitle {
                banner_col = banner_col.child(label(fill(g, sub, cx)).caption().color(fg(0.7)));
            }

            if let Some(btn_txt) = button_text {
                let action_btn = action_clone.clone();
                let b = button(fill(g, btn_txt, cx)).icon(Icon::Rocket).enabled(cx.interactive).on_click(move |g: &mut Gui| {
                    if let Some(act) = &action_btn {
                        dispatch_action(g, act, owner.as_ref());
                    }
                });
                banner_col = banner_col.child(row().justify(Justify::End).child(b));
            }

            panel().hover_effect(true).padding(0.0).fill_width().child(banner_col).into()
        }

        PanelItem::Card { title, icon: ic, children } => {
            let mut p = panel().hover_effect(true).padding(14.0).gap(10.0).fill_width();
            if let Some(t) = title {
                let icon_kind = match ic.as_deref() {
                    Some("signal") => Icon::Refresh,
                    Some("bell") => Icon::Info,
                    Some("book") => Icon::Folder,
                    _ => Icon::Package,
                };
                p = p.child(section(t, icon_kind));
            }
            let child_els: Vec<El<Gui>> = children.iter().map(|c| render(g, c, accent_color, cx)).collect();
            p.children(child_els).into()
        }

        PanelItem::Grid { columns, gap, children } => {
            let cols = (*columns).max(1);
            let mut grid_rows: Vec<El<Gui>> = Vec::new();
            for chunk in children.chunks(cols) {
                let mut r = row().fill_width().gap(*gap);
                for c in chunk {
                    r = r.child(render(g, c, accent_color, cx).grow(1.0));
                }
                grid_rows.push(r.into());
            }
            column().fill_width().gap(*gap).children(grid_rows).into()
        }

        PanelItem::Row { gap, children } => {
            let child_els: Vec<El<Gui>> = children
                .iter()
                .map(|c| {
                    let el = render(g, c, accent_color, cx);
                    if matches!(c, PanelItem::Column { .. }) {
                        el.grow(1.0).min_width(0.0)
                    } else {
                        el
                    }
                })
                .collect();
            row().fill_width().align(Align::Center).gap(*gap).children(child_els).into()
        }

        PanelItem::Column { gap, children } => {
            let child_els: Vec<El<Gui>> = children.iter().map(|c| render(g, c, accent_color, cx)).collect();
            column().fill_width().gap(*gap).children(child_els).into()
        }

        PanelItem::Text { text, size, weight, color, truncate } => {
            let mut lbl = label(fill(g, text, cx)).font_size(*size);
            if weight.as_deref() == Some("bold") {
                lbl = lbl.weight(700);
            }
            let clr = match color.as_deref() {
                Some("accent") => accent_color,
                Some("secondary") => fg(0.55),
                _ => fg(0.9),
            };
            lbl = lbl.color(clr);
            if *truncate {
                lbl = lbl.truncate();
            }
            lbl.into()
        }

        PanelItem::Markdown { content } => {
            let content = fill(g, content, cx);
            let lines: Vec<El<Gui>> = content
                .lines()
                .map(|line| {
                    if line.starts_with("### ") {
                        label(&line[4..]).weight(700).color(fg(0.95)).into()
                    } else if line.starts_with("- ") {
                        row().gap(6.0).child(icon(Icon::Check).icon_size(12.0).color(accent_color)).child(label(&line[2..]).color(fg(0.85))).into()
                    } else {
                        label(line).color(fg(0.8)).into()
                    }
                })
                .collect();
            column().gap(6.0).fill_width().children(lines).into()
        }

        PanelItem::Button { text, icon: ic, action, .. } => {
            let icon_kind = match ic.as_deref() {
                Some("copy") => Icon::Plus,
                Some("message") => Icon::Folder,
                Some("globe") => Icon::Globe,
                Some("doc") => Icon::Doc,
                _ => Icon::Play,
            };
            let act = action.clone();
            button(fill(g, text, cx))
                .icon(icon_kind)
                .enabled(cx.interactive)
                .on_click(move |g: &mut Gui| {
                    if let Some(a) = &act {
                        dispatch_action(g, a, owner.as_ref());
                    }
                })
                .into()
        }

        PanelItem::Divider => divider().fill_width().into(),

        PanelItem::Progress { value, max, label: lbl, bind } => {
            let bound = bind.as_ref().and_then(|key| cx.plugin?.data?.get(key)?.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
            let ratio = (bound.unwrap_or(*value) / max).clamp(0.0, 1.0);
            let mut col = column().fill_width().gap(4.0);
            if let Some(l) = lbl {
                col = col.child(label(fill(g, l, cx)).caption().color(fg(0.65)));
            }
            col.child(progress_bar(Some(ratio)).fill_width().height(6.0)).into()
        }

        PanelItem::Badge { text, color: _ } => row().padding(Insets::symmetric(2.0, 6.0)).child(label(fill(g, text, cx)).caption().color(accent_color)).into(),

        PanelItem::Image { url, height, radius } => {
            let url = fill(g, url, cx);
            // The view is rebuilt on every change. Ask for the picture only while it is missing: a loaded one stays in
            // the model as long as a view names it (see `view_images`). Asking every time would decode it again on each
            // rebuild, and every decode that finishes causes another rebuild.
            if !g.m.icons.contains_key(&url) {
                g.bridge.load_online_image(&url);
            }
            image_box(g.icon(&url), height.unwrap_or(180.0), radius.unwrap_or(8.0)).fill_width().into()
        }

        PanelItem::Segmented { options, selected } => {
            let track = super::controls::seg_track();
            let mut tabs = Vec::new();
            for (idx, opt) in options.iter().enumerate() {
                let act = opt.action.clone();
                let owner = owner.clone();
                let is_sel = idx == *selected;
                let t = tab(fill(g, &opt.label, cx)).selected(is_sel);
                let t = if cx.interactive {
                    t.on_click(move |g: &mut Gui| {
                        if let Some(a) = &act {
                            dispatch_action(g, a, owner.as_ref());
                        }
                    })
                } else {
                    t
                };
                tabs.push(t);
            }
            track.options(tabs).into()
        }

        PanelItem::Toggle { label: text, key, command } => {
            let on = cx.plugin.and_then(|p| p.data?.get(key)).is_some_and(|v| v == "true");
            let (key, command) = (key.clone(), command.clone());
            checkbox(on)
                .label(fill(g, text, cx))
                .enabled(cx.interactive && owner.is_some())
                .on_change(move |g: &mut Gui, value| match &owner {
                    Some(Owner::Widget { plugin, widget }) => g.bridge.run_widget_input(&mut g.m, plugin, widget, &command, &key, value),
                    Some(Owner::Panel { plugin, panel }) => g.bridge.run_panel_input(&mut g.m, plugin, panel, &command, &key, value),
                    None => {}
                })
                .into()
        }

        PanelItem::Input { key, command, placeholder, button: submit } => {
            let draft = owner.as_ref().map(|o| o.draft(key));
            let text = draft
                .as_ref()
                .and_then(|d| g.m.plugins.drafts.get(d).cloned())
                .or_else(|| cx.plugin.and_then(|p| p.data?.get(key).cloned()))
                .unwrap_or_default();
            let mut field = text_input(text).fill_width();
            if let Some(hint) = placeholder {
                field = field.placeholder(fill(g, hint, cx));
            }
            let (Some(draft), true) = (draft, cx.interactive) else { return field.into() };
            let (key, command) = (key.clone(), command.clone());
            let send = Rc::new(move |g: &mut Gui, value: String| {
                let Some(owner) = &owner else { return };
                g.m.plugins.drafts.remove(&owner.draft(&key));
                match owner {
                    Owner::Widget { plugin, widget } => g.bridge.run_widget_input(&mut g.m, plugin, widget, &command, &key, value),
                    Owner::Panel { plugin, panel } => g.bridge.run_panel_input(&mut g.m, plugin, panel, &command, &key, value),
                }
            });
            let typed = draft.clone();
            field = field.on_change(move |g: &mut Gui, value| {
                g.m.plugins.drafts.insert(typed.clone(), value);
            });
            let enter = send.clone();
            field = field.on_submit(move |g: &mut Gui, value| enter(g, value));
            match submit {
                Some(caption) => row()
                    .fill_width()
                    .gap(8.0)
                    .align(Align::Center)
                    .child(field.grow(1.0).min_width(0.0))
                    .child(button(fill(g, caption, cx)).on_click(move |g: &mut Gui| {
                        let value = g.m.plugins.drafts.get(&draft).cloned().unwrap_or_default();
                        send(g, value);
                    }))
                    .into(),
                None => field.into(),
            }
        }

    }
}

fn dispatch_action(g: &mut Gui, action: &PanelAction, owner: Option<&Owner>) {
    match action {
        PanelAction::Launch { instance, server } => {
            let inst_id = instance.clone().or_else(|| g.m.selected.clone());
            if let Some(id) = inst_id {
                let _ = server;
                g.bridge.launch(&mut g.m, &id);
            }
        }
        PanelAction::OpenUrl { url } => {
            g.bridge.open_url(url);
        }
        PanelAction::OpenSheet { sheet } => {
            // Goes through `open`, so data such as the Java list loads only when settings are opened.
            match sheet.as_str() {
                "settings" => g.open(Sheet::Settings),
                "extensions" => super::extensions::open(g, None),
                "instances" => g.open(Sheet::Instances),
                "content" => g.open(Sheet::Library),
                "accounts" => g.open(Sheet::Accounts),
                _ => g.close(),
            }
        }
        PanelAction::Copy { text } => {
            g.copy_text(&text);
        }
        PanelAction::OpenPanel { panel } => match owner {
            // A plugin's own widgets and panels open that plugin's panels.
            Some(owner) => {
                if g.m.plugins.panel(owner.plugin(), panel).is_some() {
                    g.open(Sheet::PluginPanel(owner.plugin().to_string(), panel.clone()));
                }
            }
            None => {
                if g.m.panels.iter().any(|p| &p.id == panel) {
                    g.open(Sheet::CustomPanel(panel.clone()));
                }
            }
        },
        PanelAction::Command { command, args } => match owner {
            Some(Owner::Widget { plugin, widget }) => g.bridge.run_widget_command(&mut g.m, plugin, widget, command, args.as_ref()),
            Some(Owner::Panel { plugin, panel }) => g.bridge.run_panel_command(&mut g.m, plugin, panel, command, args.as_ref()),
            None => {}
        },
    }
}


pub struct ImageBox {
    style: Style,
    key: Option<u64>,
    image: Option<Arc<Image>>,
    height: f32,
    radius: f32,
}

pub fn image_box(image: Option<Arc<Image>>, height: f32, radius: f32) -> ImageBox {
    ImageBox { style: Style::default(), key: None, image, height, radius }
}

into_el!(ImageBox);

impl<S: 'static> Widget<S> for ImageBox {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let same_img = match (&self.image, &new.image) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        let change = if new.height != self.height {
            Change::LAYOUT
        } else if new.radius != self.radius || !same_img {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.image = new.image;
        self.height = new.height;
        self.radius = new.radius;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        let w = input.known_width.unwrap_or_else(|| match input.available_width {
            cairn_ui::AvailableSpace::Definite(w) => w,
            _ => 0.0,
        });
        Size::new(w, self.height)
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.content();
        let r = self.radius.min(b.height / 2.0).min(b.width / 2.0);
        p.fill(b, r, &Brush::Solid(fg(0.06)));
        if let Some(img) = &self.image {
            let iw = img.width() as f32;
            let ih = img.height() as f32;
            if iw > 0.0 && ih > 0.0 {
                let scale = (b.width / iw).max(b.height / ih);
                let dw = (iw * scale).round();
                let dh = (ih * scale).round();
                let dest = Rect::new(b.x + (b.width - dw) / 2.0, b.y + (b.height - dh) / 2.0, dw, dh);
                p.push_rounded_clip(b, r);
                p.draw_image(img, dest, 1.0);
                p.pop_clip();
            }
        }
    }
}
