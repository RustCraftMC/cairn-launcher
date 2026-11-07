//! Panels, title bar and dock. Each panel (`sheet`) shares one frame: header, body, footer.
//! Panels carry no glass: rows use `list_item`, controls are plain translucent widgets.

pub(crate) mod accent_picker;
mod accounts;
mod controls;
mod crash;
pub mod custom_panel;
mod dock;
pub mod dropdown;
pub(crate) mod extensions;
pub mod format;
mod instance_menu;
mod instances;
mod legal;
mod library;
mod loader_plan;
mod loaders;
mod memory;
mod notifications;
mod pack;
mod settings;
mod setup;
mod tasks;
mod title_bar;
mod update;
mod version;

use cairn_bridge::Status;
use cairn_i18n::tr;
use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, column, divider, icon, icon_button, label, panel, reveal, row, spinner};

use crate::gui::{Gui, Sheet};
use crate::style::fg;

pub use dock::{describe, dock, launch_control};
pub use dropdown::view as dropdown_view;
pub use instance_menu::view as instance_menu;
pub use loaders::instance_icon;
pub use notifications::{WIDTH as NOTIFICATION_WIDTH, banners as notification_banners};
pub use tasks::popover as tasks_popover;
pub use title_bar::title_bar;

/// Transition for switching sub-pages inside a panel, such as category or instance tabs.
/// Rises 8 px while fading in over 200 ms.
/// Clipped to the content area so it never slides over the title bar.
pub fn page_reveal(key: impl std::hash::Hash, child: impl Into<El<Gui>>) -> El<Gui> {
    reveal().key(key).grow(1.0).min_height(0.0).fill_width().clip(true).duration(std::time::Duration::from_millis(200)).offset(8.0).child(child).into()
}

/// Like [`page_reveal`], but the old page fades out first.
/// When `leaving` is true the leave animation plays, then `arrived` swaps in the new page.
/// See [`crate::gui::Paged`].
pub fn page_switch(key: impl std::hash::Hash, leaving: bool, arrived: impl Fn(&mut Gui) + 'static, child: impl Into<El<Gui>>) -> El<Gui> {
    reveal()
        .key(key)
        .grow(1.0)
        .min_height(0.0)
        .fill_width()
        .clip(true)
        .duration(std::time::Duration::from_millis(200))
        .exit_duration(std::time::Duration::from_millis(140))
        .offset(8.0)
        .leaving(leaving)
        .on_left(arrived)
        .child(child)
        .into()
}

/// Fades in a different set of row-end controls, instead of popping in.
/// For example, Delete turning into a confirm prompt with Delete and Cancel.
/// `key` must change with the state, or the new set does not fade in again.
pub fn swap(key: impl std::hash::Hash, child: impl Into<El<Gui>>) -> El<Gui> {
    reveal().fade().duration(std::time::Duration::from_millis(140)).key(key).child(child).into()
}

/// A list row. Once marked as [`crate::gui::Removal`] it collapses first, with the rows below
/// sliding up, and the removal only runs after the collapse.
pub fn removable(g: &Gui, r: crate::gui::Removal, row: impl Into<El<Gui>>) -> El<Gui> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.hash(&mut h);
    let open = !g.is_removing(&r);
    cairn_ui::collapse(open).no_appear().key(("removable", h.finish())).fill_width().on_collapsed(move |g: &mut Gui| g.finish_removal(&r)).child(row).into()
}

/// Width reserved on the right of a scroll area for its scrollbar: `bar_width_hover` plus twice
/// `bar_inset` in the cairn-ui theme.
/// Controls outside the scroll area that must line up with its content reserve the same width.
pub const SCROLL_GUTTER: f32 = 14.0;

pub fn sheet(g: &Gui, s: Sheet) -> El<Gui> {
    if g.m.status != Status::Ready {
        let body = match &g.m.status {
            Status::Failed(e) => empty_state(Icon::Warning, tr!("common.init_failed"), e),
            _ => column().fill_width().grow(1.0).align(Align::Center).justify(Justify::Center).child(spinner()).into(),
        };
        return frame(header(label("CairnLauncher").title(), Vec::new()), body, None);
    }
    match s {
        Sheet::Library => library::view(g),
        Sheet::Instances => instances::list(g),
        Sheet::NewInstance => instances::new_instance(g),
        Sheet::Version => version::view(g),
        Sheet::Accounts => accounts::view(g),
        Sheet::Settings => settings::view(g),
        Sheet::ImportPack => pack::view(g),
        Sheet::InitialSetup => setup::initial_setup_view(g),
        Sheet::ImportVersions => setup::import_versions_view(g),
        Sheet::Eula => setup::eula_panel_view(g),
        Sheet::CustomPanel(id) => custom_panel::view(g, &id),
        Sheet::PluginPanel(plugin, panel) => custom_panel::plugin_view(g, &plugin, &panel),
        Sheet::Crash(id) => crash::view(g, id),
        Sheet::Update => update::view(g),
    }
}

fn frame(header: El<Gui>, body: El<Gui>, footer: Option<El<Gui>>) -> El<Gui> {
    // The panel fills its fixed-height container. The body takes the remaining height, so tall
    // content scrolls inside it instead of growing the panel.
    build_frame(header, body.grow(1.0).min_height(0.0), footer, true)
}

/// Panel frame that sizes to its content, for small panels such as the import dialog.
fn frame_auto(header: El<Gui>, body: El<Gui>) -> El<Gui> {
    build_frame(header, body, None, false)
}

fn build_frame(header: El<Gui>, body: El<Gui>, footer: Option<El<Gui>>, fill: bool) -> El<Gui> {
    let mut p = panel().hover_effect(false).padding(0.0).gap(0.0).align(Align::Stretch);
    if fill {
        p = p.fill_height();
    }
    p = p.child(header.shrink(0.0)).child(divider().fill_width().shrink(0.0)).child(body);
    if let Some(f) = footer {
        p = p.child(divider().fill_width().shrink(0.0)).child(f.shrink(0.0));
    }
    p.into()
}

/// Header row: the lead (title or tab bar) on the left, then actions and a close button.
fn header(lead: impl Into<El<Gui>>, actions: Vec<El<Gui>>) -> El<Gui> {
    row()
        .fill_width()
        .padding(Insets::new(12.0, 12.0, 12.0, 18.0))
        .gap(8.0)
        .child(row().grow(1.0).min_width(0.0).gap(6.0).child(lead))
        .children(actions)
        .child(icon_button(Icon::Close).label(tr!("common.close")).on_click(|g: &mut Gui| g.close_current()))
        .into()
}

fn section(title: &str, ic: Icon) -> El<Gui> {
    row().gap(8.0).padding(Insets::new(6.0, 0.0, 2.0, 2.0)).child(icon(ic).icon_size(14.0)).child(label(title).secondary().weight(700)).into()
}

fn empty_state(ic: Icon, title: &str, detail: &str) -> El<Gui> {
    column()
        .fill_width()
        .grow(1.0)
        .align(Align::Center)
        .justify(Justify::Center)
        .gap(8.0)
        .padding(32.0)
        .child(icon(ic).icon_size(30.0).color(fg(0.35)))
        .child(label(title).weight(700))
        .child(label(detail).caption())
        .into()
}

fn loading_row(text: &str) -> El<Gui> {
    row().fill_width().justify(Justify::Center).gap(10.0).padding(18.0).child(spinner()).child(label(text).caption()).into()
}
