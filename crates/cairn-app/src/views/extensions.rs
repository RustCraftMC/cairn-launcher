//! Management of local extensions: YAML widgets and panels (without changing their files) and
//! plugin packages, including the permissions a plugin is granted when it is enabled.
//! It is the Plugins tab of the settings panel.

use cairn_bridge::{
    Blocked, ExtensionContent, ExtensionEntry, ExtensionIssue, ExtensionKind, ExtensionPermission, Permission, PluginEntry, PluginLogLevel, RunState,
};
use cairn_i18n::tr;
use cairn_ui::{
    Align, El, Icon, Insets, Rect, Styled, button, checkbox, column, divider, icon, label, list_item, menu, menu_item, row, scroll, slider, tab, text_input,
};

use super::controls::{dropdown_button, segmented, setting_row};
use super::custom_panel::{PluginScope, Scope, Surface, render};
use super::{empty_state, page_switch};
use crate::gui::{DropdownKind, Gui, Paged, SettingsTab, Sheet};
use crate::style::{AMBER, GREEN, RED, accent, fg};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Installed,
    Widgets,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Detail {
    #[default]
    Overview,
    Permissions,
    Versions,
    Settings,
    Logs,
}

#[derive(Debug, Default)]
pub struct State {
    pub tab: Paged<Tab>,
    pub detail: Paged<Detail>,
    pub selected: Option<String>,
    /// Plugin waiting for the user to confirm its permissions before it is enabled.
    pub confirm: Option<String>,
}

/// Opens the manager, with a YAML widget selected if `widget_id` is given.
pub fn open(g: &mut Gui, widget_id: Option<&str>) {
    if let Some(id) = widget_id {
        g.ui.extensions.selected =
            g.m.extensions.entries.iter().find(|entry| entry.kind == ExtensionKind::Widget && entry.id.as_deref() == Some(id)).map(|entry| entry.key.clone());
        g.ui.extensions.tab = Paged::new(Tab::Installed);
        g.ui.extensions.detail = Paged::default();
    }
    show(g);
}

/// Opens the manager with a plugin selected.
pub fn open_plugin(g: &mut Gui, plugin: &str) {
    g.ui.extensions.selected = g.m.plugins.entry(plugin).map(plugin_key);
    g.ui.extensions.tab = Paged::new(Tab::Installed);
    g.ui.extensions.detail = Paged::default();
    show(g);
}

/// Brings up the Plugins tab of the settings panel.
fn show(g: &mut Gui) {
    g.ui.settings_tab = Paged::new(SettingsTab::Extensions);
    g.open(Sheet::Settings);
}

fn plugin_key(entry: &PluginEntry) -> String {
    format!("plugin:{}", entry.folder())
}

fn reload_all(g: &mut Gui) {
    g.bridge.reload_panels(&mut g.m);
    g.bridge.reload_plugins(&mut g.m, true);
}

fn kind_label(kind: ExtensionKind) -> &'static str {
    match kind {
        ExtensionKind::Widget => tr!("extensions.yaml_widget"),
        ExtensionKind::Panel => tr!("extensions.yaml_panel"),
    }
}

fn status(g: &Gui, entry: &ExtensionEntry) -> &'static str {
    if entry.issue.is_some() || g.m.extensions.error.is_some() {
        tr!("extensions.needs_attention")
    } else if g.m.extensions.enabled(entry) {
        tr!("extensions.enabled")
    } else {
        tr!("extensions.disabled")
    }
}

/// The Plugins tab: one toolbar over the installed list or the widget gallery.
/// Everything that adds or opens something sits in the Add menu, so the toolbar has two buttons.
pub fn view(g: &Gui) -> El<Gui> {
    let ui = &g.ui.extensions;
    let pages = segmented(&[(Tab::Installed, tr!("extensions.installed")), (Tab::Widgets, tr!("extensions.widgets"))], ui.tab.target(), |g: &mut Gui, page| {
        g.ui.extensions.tab.go(page)
    });
    let toolbar = row()
        .fill_width()
        .gap(8.0)
        .align(Align::Center)
        .padding(Insets::new(10.0, 16.0, 6.0, 16.0))
        .child(pages)
        .child(row().grow(1.0))
        .child(button(tr!("extensions.reload")).icon(Icon::Refresh).enabled(!g.m.extensions.busy && !g.m.plugins.busy).on_click(reload_all))
        .child(dropdown_button(tr!("common.add"), |g: &mut Gui, at: Rect| g.open_dropdown(DropdownKind::AddExtension, at)).icon(Icon::Plus));
    let mut body = column().fill_width().grow(1.0).min_height(0.0).child(toolbar);
    if cairn_bridge::plugins_disabled() {
        body =
            body.child(column().fill_width().padding(Insets::symmetric(18.0, 4.0)).child(label(tr!("extensions.plugins_paused_hint")).caption().color(AMBER)));
    }
    if let Some(error) = g.m.extensions.error.as_ref().or(g.m.plugins.error.as_ref()) {
        body = body.child(
            column()
                .fill_width()
                .gap(3.0)
                .padding(Insets::symmetric(18.0, 6.0))
                .child(label(tr!("extensions.state_error")).color(RED))
                .child(label(error).caption()),
        );
    }
    let content = match ui.tab.shown {
        Tab::Installed => installed(g),
        Tab::Widgets => widgets(g),
    };
    body.child(page_switch(
        ("extensions", ui.tab.shown),
        ui.tab.leaving(),
        |g: &mut Gui| {
            g.ui.extensions.tab.arrive();
        },
        content,
    ))
    .into()
}

/// The menu under the Add button: the ways to bring a plugin in, then the folders to drop files into.
pub fn add_menu(g: &Gui, min_width: f32) -> El<Gui> {
    let idle = !g.m.plugins.busy;
    let mut items = menu()
        .min_width(min_width)
        .child(menu_action(tr!("extensions.new_plugin"), Icon::Plus, idle, |g: &mut Gui| g.bridge.create_plugin(&mut g.m)))
        .child(menu_action(tr!("extensions.install_zip"), Icon::Download, idle, |g: &mut Gui| g.bridge.pick_plugin_package(&mut g.m, false)))
        .child(menu_action(tr!("extensions.install_folder"), Icon::Folder, idle, |g: &mut Gui| g.bridge.pick_plugin_package(&mut g.m, true)));
    if let Some(data) = g.m.data_dir.clone() {
        let plugins = cairn_bridge::plugins_dir(&data);
        items = items
            .child(divider().fill_width().margin(Insets::symmetric(8.0, 4.0)))
            .child(menu_action(tr!("extensions.open_folder"), Icon::Folder, true, move |g: &mut Gui| g.bridge.open_path(&data)))
            .child(menu_action(tr!("extensions.plugins_folder"), Icon::Puzzle, true, move |g: &mut Gui| {
                let _ = std::fs::create_dir_all(&plugins);
                g.bridge.open_path(&plugins);
            }));
    }
    items.into()
}

/// A menu entry that closes the menu first, then runs.
fn menu_action(text: &str, icon: Icon, enabled: bool, run: impl Fn(&mut Gui) + 'static) -> El<Gui> {
    menu_item(text)
        .icon(icon)
        .enabled(enabled)
        .on_click(move |g: &mut Gui| {
            g.close_dropdown();
            run(g);
        })
        .into()
}

enum Selected<'a> {
    Yaml(&'a ExtensionEntry),
    Plugin(&'a PluginEntry),
}

fn installed(g: &Gui) -> El<Gui> {
    let entries = &g.m.extensions.entries;
    let plugins = &g.m.plugins.entries;
    let chosen = g.ui.extensions.selected.as_ref();
    let selected = plugins
        .iter()
        .find(|entry| Some(&plugin_key(entry)) == chosen)
        .map(Selected::Plugin)
        .or_else(|| entries.iter().find(|entry| Some(&entry.key) == chosen).map(Selected::Yaml))
        .or_else(|| plugins.first().map(Selected::Plugin))
        .or_else(|| entries.first().map(Selected::Yaml));
    let Some(selected) = selected else {
        return empty_state(Icon::Puzzle, tr!("extensions.empty_title"), tr!("extensions.empty_hint"));
    };
    let selected_key = match selected {
        Selected::Yaml(entry) => entry.key.clone(),
        Selected::Plugin(entry) => plugin_key(entry),
    };
    let plugin_rows = plugins.iter().map(|entry| {
        let key = plugin_key(entry);
        let name = entry.manifest.as_ref().map_or_else(|| entry.folder(), |m| m.name.clone());
        let (text, color) = plugin_status(g, entry);
        let broken = entry.issue.is_some();
        let select = key.clone();
        list_item()
            .key(("plugin", &key))
            .selected(selected_key == key)
            .padding(Insets::symmetric(10.0, 12.0))
            .gap(9.0)
            .on_click(move |g: &mut Gui| {
                g.ui.extensions.selected = Some(select.clone());
                g.ui.extensions.detail = Paged::default();
            })
            .child(icon(plugin_icon(entry)).icon_size(18.0).color(if broken { RED } else { accent() }))
            .child(column().grow(1.0).min_width(0.0).gap(2.0).child(label(name).weight(700).truncate()).child(label(plugin_kind(entry)).caption().truncate()))
            .child(label(text).caption().color(color).shrink(0.0))
    });
    let rows = entries.iter().map(|entry| {
        let key = entry.key.clone();
        list_item()
            .key(("extension", &entry.key))
            .selected(selected_key == entry.key)
            .padding(Insets::symmetric(10.0, 12.0))
            .gap(9.0)
            .on_click(move |g: &mut Gui| {
                g.ui.extensions.selected = Some(key.clone());
                g.ui.extensions.detail = Paged::default();
            })
            .child(
                icon(match entry.kind {
                    _ if entry.issue.is_some() => Icon::Warning,
                    ExtensionKind::Widget => Icon::Widgets,
                    ExtensionKind::Panel => Icon::Doc,
                })
                .icon_size(18.0)
                .color(if entry.issue.is_some() { RED } else { accent() }),
            )
            .child(
                column()
                    .grow(1.0)
                    .min_width(0.0)
                    .gap(2.0)
                    .child(label(&entry.name).weight(700).truncate())
                    .child(label(kind_label(entry.kind)).caption().truncate()),
            )
            .child(label(status(g, entry)).caption().color(if entry.issue.is_some() { RED } else { fg(0.55) }).shrink(0.0))
    });
    row()
        .grow(1.0)
        .min_height(0.0)
        .fill_width()
        .align(Align::Stretch)
        .child(
            scroll().width(280.0).shrink(0.0).align(Align::Stretch).padding(Insets::new(2.0, 10.0, 16.0, 10.0)).gap(3.0).children(plugin_rows).children(rows),
        )
        .child(divider().vertical())
        .child(match selected {
            Selected::Yaml(entry) => details(g, entry),
            Selected::Plugin(entry) => plugin_details(g, entry),
        })
        .into()
}

fn issue_text(issue: &ExtensionIssue) -> String {
    match issue {
        ExtensionIssue::Invalid(error) => tr!("extensions.invalid", error = error),
        ExtensionIssue::EmptyId => tr!("extensions.empty_id").into(),
        ExtensionIssue::Duplicate(source) => tr!("extensions.duplicate_id", source = source.display()),
    }
}

fn detail_tabs(g: &Gui, details: &[(Detail, &'static str)]) -> El<Gui> {
    row()
        .gap(2.0)
        .children(
            details.iter().map(|&(value, text)| {
                tab(text).selected(g.ui.extensions.detail.target() == value).on_click(move |g: &mut Gui| g.ui.extensions.detail.go(value))
            }),
        )
        .into()
}

fn details(g: &Gui, entry: &ExtensionEntry) -> El<Gui> {
    let key = entry.key.clone();
    let enabled = g.m.extensions.enabled(entry);
    let title = column().grow(1.0).min_width(0.0).gap(3.0).child(label(&entry.name).title().truncate()).child(label(kind_label(entry.kind)).caption());
    let toggle = checkbox(enabled)
        .label(status(g, entry))
        .enabled(!g.m.extensions.busy && g.m.extensions.error.is_none() && entry.issue.is_none())
        .on_change(move |g: &mut Gui, enabled| {
            g.bridge.set_extension_enabled(&mut g.m, &key, enabled);
        })
        .shrink(0.0);
    let tabs = detail_tabs(
        g,
        &[(Detail::Overview, tr!("extensions.overview")), (Detail::Permissions, tr!("extensions.permissions")), (Detail::Versions, tr!("extensions.versions"))],
    );
    let mut content = column().fill_width().gap(12.0);
    match g.ui.extensions.detail.shown {
        Detail::Overview => {
            if let Some(issue) = &entry.issue {
                content = content.child(label(issue_text(issue)).color(RED));
            } else if !enabled {
                content = content.child(label(tr!("extensions.disabled_hint")).caption());
            }
            if let Some(ExtensionContent::Widget(def)) = &entry.content {
                if let Some(summary) = &def.summary {
                    content = content.child(label(summary).caption());
                }
                if entry.issue.is_none() {
                    content = content.child(preview(g, &def.view));
                }
                let id = def.id.clone();
                content = content.child(row().child(
                    button(tr!("extensions.add_home")).icon(Icon::Plus).accent().enabled(enabled).on_click(move |g: &mut Gui| crate::board::add_custom(g, &id)),
                ));
            }
            if let Some(id) = &entry.id {
                content = content.child(label(tr!("extensions.identifier", id = id)).caption());
            }
            let path = entry.source.clone();
            content = content
                .child(label(tr!("extensions.source", source = path.display())).caption())
                .child(row().child(button(tr!("extensions.show_file")).icon(Icon::Folder).on_click(move |g: &mut Gui| g.bridge.open_path(&path))));
        }
        Detail::Permissions => {
            if entry.content.is_none() {
                content = content.child(label(tr!("extensions.permissions_unavailable")).caption());
            } else {
                let permissions = entry.permissions();
                if permissions.is_empty() {
                    content = content.child(label(tr!("extensions.no_actions")).caption());
                }
                for permission in permissions {
                    let text = match permission {
                        ExtensionPermission::Launch => tr!("extensions.permission_launch").into(),
                        ExtensionPermission::OpenUrl(url) => tr!("extensions.permission_url", url = url),
                        ExtensionPermission::OpenSheet(sheet) => tr!("extensions.permission_sheet", sheet = sheet),
                        ExtensionPermission::Copy => tr!("extensions.permission_copy").into(),
                    };
                    content = content.child(label(text)).child(divider().fill_width());
                }
                content = content.child(label(tr!("extensions.permissions_hint")).caption());
            }
        }
        Detail::Versions | Detail::Settings | Detail::Logs => {
            content = content.child(label(tr!("extensions.local_config")).weight(700)).child(label(tr!("extensions.local_config_hint")).caption());
        }
    }
    let content = page_switch(
        ("extension_detail", entry.key.clone(), g.ui.extensions.detail.shown),
        g.ui.extensions.detail.leaving(),
        |g: &mut Gui| {
            g.ui.extensions.detail.arrive();
        },
        scroll().grow(1.0).min_height(0.0).child(content),
    );
    column()
        .grow(1.0)
        .min_width(0.0)
        .min_height(0.0)
        .gap(12.0)
        .padding(Insets::new(12.0, 18.0, 18.0, 18.0))
        .child(row().fill_width().gap(10.0).child(title).child(toggle))
        .child(tabs)
        .child(content)
        .into()
}

fn plugin_kind(entry: &PluginEntry) -> &'static str {
    match &entry.manifest {
        Some(m) if m.main.is_none() => tr!("extensions.declarative_plugin"),
        _ => tr!("extensions.script_plugin"),
    }
}

fn plugin_status(g: &Gui, entry: &PluginEntry) -> (&'static str, cairn_ui::Color) {
    match g.m.plugins.blocked(entry) {
        Some(Blocked::Invalid) => return (tr!("extensions.needs_attention"), RED),
        Some(Blocked::NeedsApproval) => return (tr!("extensions.plugin_needs_approval"), AMBER),
        Some(Blocked::Disabled) => return (tr!("extensions.disabled"), fg(0.55)),
        None => {}
    }
    if entry.manifest.as_ref().is_some_and(|m| m.main.is_none()) {
        return (tr!("extensions.enabled"), fg(0.55));
    }
    if cairn_bridge::plugins_disabled() {
        return (tr!("extensions.plugin_paused"), AMBER);
    }
    match entry.id().and_then(|id| g.m.plugins.runtime.get(id)).map(|r| &r.state) {
        Some(RunState::Running) => (tr!("extensions.plugin_running"), GREEN),
        Some(RunState::Failed(_)) => (tr!("extensions.plugin_error"), RED),
        _ => (tr!("extensions.plugin_starting"), fg(0.55)),
    }
}

fn permission_text(permission: Permission) -> &'static str {
    match permission {
        Permission::Network => tr!("extensions.permission_network"),
        Permission::Fs => tr!("extensions.permission_fs"),
        Permission::Process => tr!("extensions.permission_process"),
        Permission::Launch => tr!("extensions.permission_patch"),
        Permission::Instances => tr!("extensions.permission_instances"),
        Permission::Game => tr!("extensions.permission_game"),
    }
}

fn plugin_icon(entry: &PluginEntry) -> Icon {
    match &entry.manifest {
        _ if entry.issue.is_some() => Icon::Warning,
        Some(m) if m.main.is_none() => Icon::Widgets,
        _ => Icon::Puzzle,
    }
}

fn permission_icon(permission: Permission) -> Icon {
    match permission {
        Permission::Network => Icon::Globe,
        Permission::Fs => Icon::Doc,
        Permission::Process => Icon::Terminal,
        Permission::Launch => Icon::Sliders,
        Permission::Instances => Icon::Stack,
        Permission::Game => Icon::Bell,
    }
}

fn permission_row(permission: Permission, state: Option<&'static str>) -> El<Gui> {
    let mut r = row()
        .fill_width()
        .gap(10.0)
        .align(Align::Center)
        .child(icon(permission_icon(permission)).icon_size(16.0).color(fg(0.7)).shrink(0.0))
        .child(label(permission_text(permission)).grow(1.0).min_width(0.0));
    if permission.high_risk() {
        r = r.child(
            row()
                .gap(4.0)
                .align(Align::Center)
                .shrink(0.0)
                .child(icon(Icon::ShieldAlert).icon_size(13.0).color(RED))
                .child(label(tr!("extensions.high_risk")).caption().color(RED)),
        );
    }
    if let Some(state) = state {
        r = r.child(label(state).caption().shrink(0.0));
    }
    r.into()
}

/// Enabling a plugin that asks for permissions the user has not granted shows a confirmation first.
fn set_plugin_enabled(g: &mut Gui, id: &str, enabled: bool) {
    let needs_confirm = enabled
        && g.m
            .plugins
            .entry(id)
            .and_then(|e| e.manifest.as_ref())
            .is_some_and(|m| !m.permissions.is_empty() && g.m.plugins.state.grant(id).is_none_or(|grant| !m.permissions.is_subset(&grant.permissions)));
    if needs_confirm {
        g.ui.extensions.confirm = Some(id.to_string());
    } else {
        g.ui.extensions.confirm = None;
        g.bridge.set_plugin_enabled(&mut g.m, id, enabled);
    }
}

fn confirm_panel(m: &cairn_bridge::Manifest) -> El<Gui> {
    let id = m.id.clone();
    column()
        .fill_width()
        .gap(8.0)
        .padding(14.0)
        .background(fg(0.05))
        .corner_radius(14.0)
        .child(
            row()
                .gap(8.0)
                .align(Align::Center)
                .child(icon(Icon::Shield).icon_size(18.0).color(accent()))
                .child(label(tr!("extensions.confirm_title", name = m.name)).weight(700)),
        )
        .children(m.permissions.iter().map(|&p| permission_row(p, None)))
        .child(label(tr!("extensions.confirm_hint")).caption())
        .child(
            row()
                .gap(8.0)
                .child(button(tr!("extensions.confirm_enable")).accent().on_click(move |g: &mut Gui| {
                    g.ui.extensions.confirm = None;
                    g.bridge.set_plugin_enabled(&mut g.m, &id, true);
                }))
                .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.ui.extensions.confirm = None)),
        )
        .into()
}

fn plugin_details(g: &Gui, entry: &PluginEntry) -> El<Gui> {
    let manifest = entry.manifest.as_ref();
    let id = entry.id().unwrap_or_default().to_string();
    let name = manifest.map_or_else(|| entry.folder(), |m| m.name.clone());
    let blocked = g.m.plugins.blocked(entry);
    let runtime = g.m.plugins.runtime.get(&id);
    let title = column().grow(1.0).min_width(0.0).gap(3.0).child(label(&name).title().truncate()).child(label(plugin_kind(entry)).caption());
    let toggle_id = id.clone();
    let toggle = checkbox(!matches!(blocked, Some(Blocked::Disabled | Blocked::Invalid)))
        .label(plugin_status(g, entry).0)
        .enabled(!g.m.plugins.busy && g.m.plugins.error.is_none() && entry.issue.is_none())
        .on_change(move |g: &mut Gui, enabled| set_plugin_enabled(g, &toggle_id, enabled))
        .shrink(0.0);
    let mut head = column().fill_width().gap(10.0).child(row().fill_width().gap(10.0).child(title).child(toggle));
    if let Some(m) = manifest.filter(|_| g.ui.extensions.confirm.as_deref() == Some(id.as_str())) {
        head = head.child(confirm_panel(m));
    }
    let mut details = vec![(Detail::Overview, tr!("extensions.overview")), (Detail::Permissions, tr!("extensions.permissions"))];
    if manifest.is_some_and(|m| !m.settings.is_empty()) {
        details.push((Detail::Settings, tr!("extensions.settings_tab")));
    }
    details.extend([(Detail::Versions, tr!("extensions.versions")), (Detail::Logs, tr!("extensions.logs"))]);
    let tabs = detail_tabs(g, &details);
    let mut content = column().fill_width().gap(12.0);
    match g.ui.extensions.detail.shown {
        Detail::Overview => {
            if let Some(issue) = &entry.issue {
                content = content.child(label(tr!("extensions.invalid", error = issue)).color(RED));
            }
            if let Some(RunState::Failed(error)) = runtime.map(|r| &r.state) {
                content = content.child(label(error).color(RED));
            }
            if blocked == Some(Blocked::Disabled) {
                content = content.child(label(tr!("extensions.disabled_hint")).caption());
            }
            if let Some(description) = manifest.and_then(|m| m.description.as_ref()) {
                content = content.child(label(description));
            }
            if let Some(r) = runtime.filter(|r| r.state == RunState::Running) {
                if r.launch_hook {
                    content = content.child(
                        row()
                            .gap(6.0)
                            .align(Align::Center)
                            .child(icon(Icon::Sliders).icon_size(14.0).color(accent()))
                            .child(label(tr!("extensions.launch_hook_active")).caption().color(accent())),
                    );
                }
                if let Some(ms) = r.activate_ms {
                    content = content.child(label(tr!("extensions.activate_time", ms = format!("{ms:.1}"))).caption());
                }
            }
            let commands = manifest.map_or(&[][..], |m| m.commands.as_slice());
            if !commands.is_empty() {
                let running = runtime.is_some_and(|r| r.state == RunState::Running);
                content = content.child(super::section(tr!("extensions.commands"), Icon::Terminal));
                for command in commands {
                    let (plugin, cmd) = (id.clone(), command.id.clone());
                    content = content.child(
                        row().fill_width().gap(8.0).align(Align::Center).child(label(&command.title).grow(1.0).min_width(0.0).truncate()).child(
                            button(tr!("extensions.run_command"))
                                .icon(Icon::Play)
                                .enabled(running)
                                .on_click(move |g: &mut Gui| g.bridge.run_plugin_command(&mut g.m, &plugin, &cmd, Default::default())),
                        ),
                    );
                }
            }
            if let Ok(defs) = cairn_bridge::Plugins::declared_panels(entry)
                && !defs.is_empty()
            {
                content = content.child(super::section(tr!("extensions.plugin_panels"), Icon::Doc));
                for def in defs {
                    let (plugin, panel) = (id.clone(), def.id.clone());
                    content = content.child(
                        row().fill_width().gap(8.0).align(Align::Center).child(label(&def.title).grow(1.0).min_width(0.0).truncate()).child(
                            button(tr!("extensions.open_panel"))
                                .icon(Icon::Doc)
                                .enabled(blocked.is_none())
                                .on_click(move |g: &mut Gui| g.open(Sheet::PluginPanel(plugin.clone(), panel.clone()))),
                        ),
                    );
                }
            }
            if let (Some(m), Ok(defs)) = (manifest, cairn_bridge::Plugins::declared_widgets(entry))
                && !defs.is_empty()
            {
                content = content.child(super::section(tr!("extensions.plugin_widgets"), Icon::Widgets));
                if blocked.is_some() {
                    content = content.child(label(tr!("extensions.enable_first")).caption());
                }
                for def in defs {
                    let kind = m.widget_kind(&def.id);
                    content = content.child(label(&def.name).caption()).child(plugin_preview(g, &m.id, &def)).child(
                        row().child(
                            button(tr!("extensions.add_home"))
                                .icon(Icon::Plus)
                                .accent()
                                .enabled(blocked.is_none())
                                .on_click(move |g: &mut Gui| crate::board::add_kind(g, &kind)),
                        ),
                    );
                }
            }
            if !id.is_empty() {
                content = content.child(label(tr!("extensions.identifier", id = id)).caption());
            }
            let dir = entry.dir.clone();
            content = content
                .child(label(tr!("extensions.source", source = dir.display())).caption())
                .child(row().child(button(tr!("extensions.show_file")).icon(Icon::Folder).on_click(move |g: &mut Gui| g.bridge.open_path(&dir))));
        }
        Detail::Permissions => match manifest {
            None => content = content.child(label(tr!("extensions.permissions_unavailable")).caption()),
            Some(m) => {
                if m.permissions.is_empty() {
                    content = content.child(label(tr!("extensions.no_permissions")).caption());
                }
                let grant = g.m.plugins.state.grant(&m.id);
                for &permission in &m.permissions {
                    let granted = grant.is_some_and(|grant| grant.permissions.contains(&permission));
                    let state = if granted { tr!("extensions.granted") } else { tr!("extensions.not_granted") };
                    content = content.child(permission_row(permission, Some(state))).child(divider().fill_width());
                }
                content = content.child(label(tr!("extensions.always_allowed")).caption());
            }
        },
        Detail::Versions => match manifest {
            None => content = content.child(label(tr!("extensions.permissions_unavailable")).caption()),
            Some(m) => {
                content = content.child(label(tr!("extensions.version_label", version = m.version)).weight(700)).child(
                    row()
                        .gap(6.0)
                        .align(Align::Center)
                        .child(icon(Icon::Braces).icon_size(13.0).color(fg(0.55)))
                        .child(label(tr!("extensions.api_version", api = m.api)).caption()),
                );
                if let Some(author) = &m.author {
                    content = content.child(label(tr!("extensions.author", author = author)).caption());
                }
                if let Some(url) = m.homepage.clone().filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
                    content = content
                        .child(row().child(button(tr!("extensions.open_homepage")).icon(Icon::Globe).on_click(move |g: &mut Gui| g.bridge.open_url(&url))));
                }
                content = content.child(divider().fill_width()).child(manage_package(g, m));
            }
        },
        Detail::Settings => match manifest {
            Some(m) => content = content.child(plugin_settings(g, m)),
            None => content = content.child(label(tr!("extensions.permissions_unavailable")).caption()),
        },
        Detail::Logs => match runtime.filter(|r| !r.logs.is_empty()) {
            None => content = content.child(label(tr!("extensions.no_logs")).caption()),
            Some(r) => {
                for log in r.logs.iter().rev() {
                    let color = match log.level {
                        PluginLogLevel::Error => RED,
                        PluginLogLevel::Warn => AMBER,
                        PluginLogLevel::Debug => fg(0.5),
                        PluginLogLevel::Info => fg(0.85),
                    };
                    content = content.child(
                        row()
                            .fill_width()
                            .gap(8.0)
                            .align(Align::Start)
                            .child(label(&log.time).caption().shrink(0.0))
                            .child(label(&log.message).color(color).grow(1.0).min_width(0.0)),
                    );
                }
            }
        },
    }
    let content = page_switch(
        ("plugin_detail", plugin_key(entry), g.ui.extensions.detail.shown),
        g.ui.extensions.detail.leaving(),
        |g: &mut Gui| {
            g.ui.extensions.detail.arrive();
        },
        scroll().grow(1.0).min_height(0.0).child(content),
    );
    column().grow(1.0).min_width(0.0).min_height(0.0).gap(12.0).padding(Insets::new(12.0, 18.0, 18.0, 18.0)).child(head).child(tabs).child(content).into()
}

/// Rollback and uninstall, with an inline confirmation before anything is deleted.
fn manage_package(g: &Gui, m: &cairn_bridge::Manifest) -> El<Gui> {
    let busy = g.m.plugins.busy;
    let mut actions = row().gap(8.0);
    if let Some(version) = g.m.plugins.backups.get(&m.id) {
        let id = m.id.clone();
        actions = actions.child(
            button(tr!("extensions.rollback_to", version = version))
                .icon(Icon::Refresh)
                .enabled(!busy)
                .on_click(move |g: &mut Gui| g.bridge.rollback_plugin(&mut g.m, &id)),
        );
    }
    let id = m.id.clone();
    actions = actions.child(
        button(tr!("extensions.uninstall")).icon(Icon::Trash).enabled(!busy).on_click(move |g: &mut Gui| g.m.plugins.uninstalling = Some((id.clone(), false))),
    );
    let mut out = column().fill_width().gap(10.0).child(actions);
    if let Some((_, data)) = g.m.plugins.uninstalling.as_ref().filter(|(id, _)| id == &m.id) {
        let id = m.id.clone();
        out = out.child(
            column()
                .fill_width()
                .gap(8.0)
                .padding(14.0)
                .background(fg(0.05))
                .corner_radius(14.0)
                .child(label(tr!("extensions.uninstall_confirm", name = m.name)).weight(700))
                .child(checkbox(*data).label(tr!("extensions.uninstall_data")).on_change(|g: &mut Gui, data| {
                    if let Some(pending) = &mut g.m.plugins.uninstalling {
                        pending.1 = data;
                    }
                }))
                .child(
                    row()
                        .gap(8.0)
                        .child(button(tr!("extensions.uninstall")).accent().enabled(!busy).on_click(move |g: &mut Gui| {
                            let data = g.m.plugins.uninstalling.as_ref().is_some_and(|(_, data)| *data);
                            g.bridge.uninstall_plugin(&mut g.m, &id, data);
                        }))
                        .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.m.plugins.uninstalling = None)),
                ),
        );
    }
    out.into()
}

/// One native control per declared setting. Changes are saved and sent to the plugin as they happen.
fn plugin_settings(g: &Gui, m: &cairn_bridge::Manifest) -> El<Gui> {
    use cairn_bridge::SettingKind;
    let values = g.m.plugins.settings.get(&m.id);
    let mut out = column().fill_width().gap(4.0);
    for decl in &m.settings {
        let value = values.and_then(|v| v.get(&decl.id)).cloned().unwrap_or_else(|| decl.default_value());
        let (plugin, key) = (m.id.clone(), decl.id.clone());
        let control: El<Gui> = match decl.kind {
            SettingKind::Boolean => checkbox(value.as_bool().unwrap_or(false))
                .on_change(move |g: &mut Gui, on| g.bridge.set_plugin_setting(&mut g.m, &plugin, &key, on.into()))
                .into(),
            SettingKind::Choice => {
                let current = decl.options.iter().position(|o| value.as_str() == Some(o.value.as_str())).unwrap_or(0);
                let items: Vec<(usize, &str)> = decl.options.iter().enumerate().map(|(i, o)| (i, o.label.as_str())).collect();
                let values: Vec<String> = decl.options.iter().map(|o| o.value.clone()).collect();
                segmented(&items, current, move |g: &mut Gui, i| g.bridge.set_plugin_setting(&mut g.m, &plugin, &key, values[i].clone().into()))
            }
            SettingKind::Number => {
                let number = value.as_f64().unwrap_or(0.0);
                match (decl.min, decl.max) {
                    (Some(min), Some(max)) => {
                        let text = if number.fract() == 0.0 { format!("{number:.0}") } else { format!("{number}") };
                        let mut track = slider(number as f32).range(min as f32, max as f32);
                        if let Some(step) = decl.step {
                            track = track.step(step as f32);
                        }
                        row()
                            .gap(10.0)
                            .align(Align::Center)
                            .child(track.width(180.0).on_change(move |g: &mut Gui, v| g.bridge.set_plugin_setting(&mut g.m, &plugin, &key, (v as f64).into())))
                            .child(label(text).width(52.0))
                            .into()
                    }
                    _ => {
                        let draft = format!("settings/{plugin}/{key}");
                        let shown = g.m.plugins.drafts.get(&draft).cloned().unwrap_or_else(|| value.to_string());
                        text_input(shown)
                            .width(140.0)
                            .on_change(move |g: &mut Gui, text: String| {
                                if let Ok(v) = text.trim().parse::<f64>() {
                                    g.bridge.set_plugin_setting(&mut g.m, &plugin, &key, v.into());
                                }
                                g.m.plugins.drafts.insert(draft.clone(), text);
                            })
                            .into()
                    }
                }
            }
            SettingKind::String => {
                let mut field = text_input(value.as_str().unwrap_or_default()).width(240.0);
                if let Some(hint) = &decl.placeholder {
                    field = field.placeholder(hint);
                }
                if decl.secret {
                    field = field.secret();
                }
                field.on_change(move |g: &mut Gui, text: String| g.bridge.set_plugin_setting(&mut g.m, &plugin, &key, text.into())).into()
            }
        };
        out = out.child(setting_row(&decl.title, decl.description.as_deref().unwrap_or_default(), control));
    }
    let id = m.id.clone();
    out.child(row().padding(Insets::new(8.0, 0.0, 0.0, 6.0)).child(button(tr!("extensions.reset_settings")).icon(Icon::Refresh).on_click(
        move |g: &mut Gui| {
            g.m.plugins.drafts.retain(|k, _| !k.starts_with(&format!("settings/{id}/")));
            g.bridge.reset_plugin_settings(&mut g.m, &id);
        },
    )))
    .into()
}

fn preview(g: &Gui, items: &[cairn_bridge::PanelItem]) -> El<Gui> {
    column()
        .fill_width()
        .min_height(80.0)
        .padding(14.0)
        .gap(8.0)
        .background(fg(0.035))
        .corner_radius(14.0)
        .children(items.iter().map(|item| super::custom_panel::render_preview(g, item, accent())))
        .into()
}

/// Previews a plugin widget with the latest data the plugin sent, if it is running.
fn plugin_preview(g: &Gui, plugin: &str, def: &cairn_bridge::CustomWidgetDef) -> El<Gui> {
    let live = g.m.plugins.live.get(&cairn_bridge::widget_kind(plugin, &def.id));
    let scope = Scope { interactive: false, plugin: Some(PluginScope { plugin, surface: Surface::Widget(&def.id), data: live.map(|l| &l.data) }) };
    let items = live.and_then(|l| l.view.as_deref()).unwrap_or(&def.view);
    column()
        .fill_width()
        .min_height(80.0)
        .padding(14.0)
        .gap(8.0)
        .background(fg(0.035))
        .corner_radius(14.0)
        .children(items.iter().map(|item| render(g, item, accent(), scope)))
        .into()
}

fn widget_card(name: &str) -> cairn_ui::Flex<Gui> {
    column().width(270.0).padding(14.0).gap(10.0).background(fg(0.035)).corner_radius(14.0).child(label(name).weight(700))
}

fn manage_button(key: String) -> El<Gui> {
    button(tr!("extensions.manage_owner"))
        .text_only()
        .on_click(move |g: &mut Gui| {
            g.ui.extensions.selected = Some(key.clone());
            g.ui.extensions.detail = Paged::default();
            g.ui.extensions.tab.go(Tab::Installed);
        })
        .into()
}

fn widgets(g: &Gui) -> El<Gui> {
    let entries: Vec<_> = g.m.extensions.entries.iter().filter(|entry| entry.kind == ExtensionKind::Widget).collect();
    let mut cards = Vec::new();
    for entry in &g.m.plugins.entries {
        let (Some(m), Ok(defs)) = (&entry.manifest, cairn_bridge::Plugins::declared_widgets(entry)) else { continue };
        let blocked = g.m.plugins.blocked(entry);
        for def in defs {
            let kind = m.widget_kind(&def.id);
            cards.push(
                widget_card(&def.name)
                    .child(label(&m.name).caption())
                    .child(plugin_preview(g, &m.id, &def))
                    .child(
                        button(tr!("extensions.add_home"))
                            .icon(Icon::Plus)
                            .accent()
                            .enabled(blocked.is_none())
                            .on_click(move |g: &mut Gui| crate::board::add_kind(g, &kind)),
                    )
                    .child(manage_button(plugin_key(entry))),
            );
        }
    }
    if entries.is_empty() && cards.is_empty() {
        return empty_state(Icon::Widgets, tr!("extensions.no_widgets"), tr!("extensions.empty_hint"));
    }
    for entry in entries {
        let mut card = widget_card(&entry.name).child(label(status(g, entry)).caption());
        if let Some(issue) = &entry.issue {
            card = card.child(label(issue_text(issue)).caption().color(RED));
        }
        if entry.issue.is_none()
            && let Some(ExtensionContent::Widget(def)) = &entry.content
        {
            let id = def.id.clone();
            card = card.child(preview(g, &def.view)).child(
                button(tr!("extensions.add_home"))
                    .icon(Icon::Plus)
                    .accent()
                    .enabled(g.m.extensions.enabled(entry))
                    .on_click(move |g: &mut Gui| crate::board::add_custom(g, &id)),
            );
        }
        cards.push(card.child(manage_button(entry.key.clone())));
    }
    scroll()
        .grow(1.0)
        .min_height(0.0)
        .padding(Insets::new(6.0, 16.0, 20.0, 16.0))
        .gap(12.0)
        .child(label(tr!("extensions.builtins_hint")).caption())
        .child(label(tr!("extensions.drop_hint")).caption())
        .child(row().fill_width().align(Align::Start).wrap(true).gap(14.0).children(cards))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_bridge::{Bridge, CustomWidgetDef, Language, PanelAction, PanelItem, Remote, Sink, Status, ThemeMode};
    use cairn_ui::testing::Headless;
    use std::sync::Arc;
    use std::time::Duration;

    fn settle(app: &mut Headless<Gui>) {
        for _ in 0..8 {
            app.run_animations(120);
            app.advance(Duration::from_millis(100));
            app.render();
            if !app.animating() && !app.redraw_requested() {
                break;
            }
        }
    }

    fn fixture(rt: &tokio::runtime::Runtime, light: bool) -> Gui {
        let sink: Sink = Arc::new(|_| true);
        let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        g.m.status = Status::Ready;
        g.m.config.eula_accepted = true;
        g.m.config.setup_completed = true;
        g.m.instances = Remote::Ready(vec![]);
        g.m.data_dir = Some(std::path::PathBuf::from("C:/Cairn/.cairn"));
        g.m.appearance.mode = if light { ThemeMode::Light } else { ThemeMode::Dark };
        let def = CustomWidgetDef {
            id: "clock".into(),
            name: "Clock YAML".into(),
            summary: Some("Local widget".into()),
            icon: Some("clock".into()),
            size: (4, 2),
            sizes: vec![],
            min: (1, 1),
            max: (12, 6),
            heading: true,
            view: vec![
                PanelItem::Text { text: "{time}".into(), size: 32.0, weight: None, color: None, truncate: false },
                PanelItem::Column {
                    gap: 8.0,
                    children: vec![PanelItem::Button {
                        text: "Preview action".into(),
                        icon: None,
                        style: None,
                        action: Some(PanelAction::OpenSheet { sheet: "accounts".into() }),
                    }],
                },
            ],
        };
        g.m.widgets.push(def.clone());
        g.m.extensions.entries = vec![
            ExtensionEntry {
                key: "widgets/clock.yaml".into(),
                source: std::path::PathBuf::from("C:/Cairn/.cairn/widgets/clock.yaml"),
                kind: ExtensionKind::Widget,
                id: Some("clock".into()),
                name: def.name.clone(),
                content: Some(ExtensionContent::Widget(def)),
                issue: None,
            },
            ExtensionEntry {
                key: "widgets/broken.yaml".into(),
                source: std::path::PathBuf::from("C:/Cairn/.cairn/widgets/broken.yaml"),
                kind: ExtensionKind::Widget,
                id: None,
                name: "broken.yaml".into(),
                content: None,
                issue: Some(ExtensionIssue::Invalid("view: invalid type".into())),
            },
        ];
        g.ui.sheet = Some(Sheet::Settings);
        g.ui.settings_tab = Paged::new(SettingsTab::Extensions);
        g
    }

    fn plugin(json: &str) -> PluginEntry {
        let manifest = cairn_bridge::Manifest::parse(json.as_bytes()).unwrap();
        PluginEntry { dir: std::path::PathBuf::from(format!("C:/Cairn/.cairn/plugins/{}", manifest.id)), manifest: Some(manifest), issue: None }
    }

    #[test]
    fn manager_navigation_and_previews_work_in_all_locales_and_themes() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            cairn_i18n::with_language(language, || {
                for light in [false, true] {
                    let g = fixture(&rt, light);
                    let theme = crate::style::theme(&g.m.appearance, None, false);
                    let mut app = Headless::with_theme(g, Gui::view, 960.0, 600.0, 1.0, theme, text.take().unwrap());
                    settle(&mut app);
                    // The settings panel is 880 wide and centered in the 960 wide window.
                    let (left, right) = (40.0, 920.0);
                    let tabs = [
                        tr!("settings.appearance"),
                        tr!("settings.game"),
                        tr!("settings.java"),
                        tr!("settings.download"),
                        tr!("settings.extensions"),
                        tr!("settings.about"),
                    ]
                    .map(|name| app.find(name).unwrap_or_else(|| panic!("{language:?}: settings tab {name}")).bounds);
                    let close = app.find(tr!("common.close")).expect("close button").bounds;
                    assert!(tabs[0].x >= left && tabs.windows(2).all(|pair| pair[0].right() <= pair[1].x + 0.5), "{language:?}: tabs overlap: {tabs:?}");
                    assert!(tabs[5].right() <= close.x, "{language:?}: the last tab runs under the close button: {tabs:?} / {close:?}");
                    assert!(tabs.iter().all(|tab| (tab.height - tabs[0].height).abs() < 1.0), "{language:?}: a tab wrapped: {tabs:?}");
                    for title in [tr!("extensions.installed"), tr!("extensions.widgets"), tr!("extensions.reload"), tr!("common.add")] {
                        let node = app.find(title).expect("manager toolbar control");
                        assert!(node.bounds.x >= left && node.bounds.right() <= right && node.bounds.width > 0.0, "{language:?}: {title}: {:?}", node.bounds);
                    }
                    let entries = [
                        tr!("extensions.new_plugin"),
                        tr!("extensions.install_zip"),
                        tr!("extensions.install_folder"),
                        tr!("extensions.open_folder"),
                        tr!("extensions.plugins_folder"),
                    ];
                    assert!(entries.iter().all(|title| app.find(title).is_none()), "{language:?}: the Add menu entries stay hidden until it is opened");
                    app.click(app.find(tr!("common.add")).unwrap().bounds.center());
                    settle(&mut app);
                    for title in entries {
                        let node = app.find(title).unwrap_or_else(|| panic!("{language:?}: Add menu entry {title}"));
                        assert!(!node.semantics.disabled, "{language:?}: {title}");
                        assert!(
                            node.bounds.x >= left && node.bounds.right() <= right && node.bounds.bottom() <= 600.0,
                            "{language:?}: {title}: {:?}",
                            node.bounds
                        );
                    }
                    if let Some(dir) = std::env::var_os("CAIRN_EXTENSIONS_PREVIEW_DIR") {
                        let dir = std::path::PathBuf::from(dir);
                        std::fs::create_dir_all(&dir).unwrap();
                        std::fs::write(dir.join(format!("{}-{light}-add-menu.png", language.code())), app.png()).unwrap();
                    }
                    // A click outside only closes the menu.
                    app.click(cairn_ui::Point::new(20.0, 300.0));
                    settle(&mut app);
                    assert!(entries.iter().all(|title| app.find(title).is_none()), "{language:?}: the Add menu did not close");
                    assert_eq!(app.state().ui.sheet, Some(Sheet::Settings));
                    if let Some(dir) = std::env::var_os("CAIRN_EXTENSIONS_PREVIEW_DIR") {
                        let dir = std::path::PathBuf::from(dir);
                        std::fs::create_dir_all(&dir).unwrap();
                        std::fs::write(dir.join(format!("{}-{light}-overview.png", language.code())), app.png()).unwrap();
                    }
                    let preview = app.find("Preview action").unwrap();
                    assert!(preview.semantics.disabled);
                    app.click(preview.bounds.center());
                    assert_eq!(app.state().ui.sheet, Some(Sheet::Settings));
                    let permissions = app.find(tr!("extensions.permissions")).unwrap().bounds;
                    app.click(permissions.center());
                    settle(&mut app);
                    assert!(app.find(&tr!("extensions.permission_sheet", sheet = "accounts")).is_some());
                    let broken = app.find("broken.yaml").unwrap().bounds;
                    app.click(broken.center());
                    settle(&mut app);
                    assert!(app.find(&tr!("extensions.invalid", error = "view: invalid type")).is_some());
                    assert!(app.find_role(cairn_ui::Role::Checkbox).iter().all(|node| node.semantics.disabled));
                    if let Some(dir) = std::env::var_os("CAIRN_EXTENSIONS_PREVIEW_DIR") {
                        let dir = std::path::PathBuf::from(dir);
                        std::fs::create_dir_all(&dir).unwrap();
                        std::fs::write(dir.join(format!("{}-{light}-diagnostic.png", language.code())), app.png()).unwrap();
                    }
                    let widgets = app.find(tr!("extensions.widgets")).unwrap().bounds;
                    app.click(widgets.center());
                    settle(&mut app);
                    assert!(app.find(tr!("extensions.manage_owner")).is_some());
                    if let Some(dir) = std::env::var_os("CAIRN_EXTENSIONS_PREVIEW_DIR") {
                        std::fs::write(std::path::PathBuf::from(dir).join(format!("{}-{light}-widgets.png", language.code())), app.png()).unwrap();
                    }
                    text = Some(app.into_text_system());
                }
            });
        }
    }

    #[test]
    fn add_menu_keeps_the_folder_shortcuts_while_a_plugin_task_runs() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        cairn_i18n::with_language(Language::ZhCn, || {
            let mut g = fixture(&rt, false);
            g.m.plugins.busy = true;
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let mut app = Headless::with_theme(g, Gui::view, 960.0, 600.0, 1.0, theme, text);
            settle(&mut app);
            assert!(app.find(tr!("extensions.reload")).unwrap().semantics.disabled, "reloading waits for the running task");
            app.click(app.find(tr!("common.add")).unwrap().bounds.center());
            settle(&mut app);
            for title in [tr!("extensions.new_plugin"), tr!("extensions.install_zip"), tr!("extensions.install_folder")] {
                assert!(app.find(title).unwrap().semantics.disabled, "{title}");
            }
            for title in [tr!("extensions.open_folder"), tr!("extensions.plugins_folder")] {
                assert!(!app.find(title).unwrap().semantics.disabled, "{title}");
            }
        });
    }

    #[test]
    fn opening_the_manager_from_elsewhere_lands_on_the_plugins_tab_with_the_entry_selected() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = fixture(&rt, false);
        g.ui.sheet = None;
        g.ui.settings_tab = Paged::new(SettingsTab::About);
        g.ui.extensions.tab = Paged::new(Tab::Widgets);
        open(&mut g, Some("clock"));
        assert_eq!(g.ui.sheet, Some(Sheet::Settings));
        assert_eq!(g.ui.settings_tab.shown, SettingsTab::Extensions);
        assert_eq!(g.ui.extensions.selected.as_deref(), Some("widgets/clock.yaml"));
        assert_eq!(g.ui.extensions.tab.shown, Tab::Installed);
        // Without a widget the selection and page the user left are kept.
        g.ui.settings_tab = Paged::new(SettingsTab::Game);
        g.ui.extensions.tab = Paged::new(Tab::Widgets);
        open(&mut g, None);
        assert_eq!(g.ui.settings_tab.shown, SettingsTab::Extensions);
        assert_eq!(g.ui.extensions.tab.shown, Tab::Widgets);
        assert_eq!(g.ui.extensions.selected.as_deref(), Some("widgets/clock.yaml"));
    }

    #[test]
    fn enabling_a_plugin_asks_for_its_permissions_first() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        cairn_i18n::with_language(Language::ZhCn, || {
            let mut g = fixture(&rt, false);
            g.m.plugins.entries = vec![plugin(
                r#"{"id":"demo.tools","name":"Demo tools","version":"1.2.0","api":1,"main":"main.js","permissions":["network","process"],
                    "commands":[{"id":"sync","title":"Sync now"}],
                    "widgets":[{"id":"status","name":"Server status","size":[4,2],"view":[{"element":"text","text":"Online: {data.online}"}]}],
                    "settings":[{"id":"host","title":"Server address","type":"string","default":"mc.example.com"},
                        {"id":"interval","title":"Refresh every","type":"number","min":30,"max":600,"step":30,"default":60},
                        {"id":"compact","title":"Compact layout","type":"boolean"},
                        {"id":"mode","title":"Mode","type":"choice","options":[{"value":"players","label":"Players"},{"value":"ping","label":"Ping"}]}]}"#,
            )];
            g.m.plugins.backups.insert("demo.tools".into(), "1.1.0".into());
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let mut app = Headless::with_theme(g, Gui::view, 960.0, 600.0, 1.0, theme, text);
            settle(&mut app);
            assert!(app.find("Demo tools").is_some());
            assert!(app.find("Online: –").is_some(), "unbound data shows a dash in previews");
            let save = |app: &Headless<Gui>, name: &str| {
                if let Some(dir) = std::env::var_os("CAIRN_EXTENSIONS_PREVIEW_DIR") {
                    std::fs::write(std::path::PathBuf::from(dir).join(name), app.png()).unwrap();
                }
            };
            save(&app, "zh-CN-plugin-overview.png");
            let toggle = app.find_role(cairn_ui::Role::Checkbox).into_iter().find(|n| !n.semantics.disabled).unwrap();
            app.click(toggle.bounds.center());
            settle(&mut app);
            assert_eq!(app.state().ui.extensions.confirm.as_deref(), Some("demo.tools"));
            assert!(app.find(&tr!("extensions.confirm_title", name = "Demo tools")).is_some());
            assert!(app.find(tr!("extensions.permission_process")).is_some());
            assert!(app.state().m.plugins.state.grant("demo.tools").is_none(), "nothing is granted before confirming");
            save(&app, "zh-CN-plugin-confirm.png");
            let cancel = app.find(tr!("common.cancel")).unwrap().bounds;
            app.click(cancel.center());
            settle(&mut app);
            assert_eq!(app.state().ui.extensions.confirm, None);
            let permissions = app.find(tr!("extensions.permissions")).unwrap().bounds;
            app.click(permissions.center());
            settle(&mut app);
            assert!(app.find(tr!("extensions.not_granted")).is_some());
            save(&app, "zh-CN-plugin-permissions.png");
            let settings = app.find(tr!("extensions.settings_tab")).unwrap().bounds;
            app.click(settings.center());
            settle(&mut app);
            for title in ["Server address", "Refresh every", "Compact layout", "Mode", "Players", "Ping", tr!("extensions.reset_settings")] {
                assert!(app.find(title).is_some(), "{title}");
            }
            save(&app, "zh-CN-plugin-settings.png");
            let versions = app.find(tr!("extensions.versions")).unwrap().bounds;
            app.click(versions.center());
            settle(&mut app);
            assert!(app.find(&tr!("extensions.rollback_to", version = "1.1.0")).is_some());
            let uninstall = app.find(tr!("extensions.uninstall")).unwrap().bounds;
            app.click(uninstall.center());
            settle(&mut app);
            assert!(app.find(&tr!("extensions.uninstall_confirm", name = "Demo tools")).is_some());
            assert!(app.find(tr!("extensions.uninstall_data")).is_some());
            save(&app, "zh-CN-plugin-uninstall.png");
            // At the minimum window size the confirmation continues below the fold.
            app.pointer_move(cairn_ui::Point::new(600.0, 400.0));
            app.wheel(cairn_ui::ScrollDelta::Lines { x: 0.0, y: -10.0 });
            settle(&mut app);
            let cancel = app.find(tr!("common.cancel")).unwrap().bounds;
            app.click(cancel.center());
            settle(&mut app);
            assert!(app.state().m.plugins.uninstalling.is_none(), "cancel deletes nothing");
            let logs = app.find(tr!("extensions.logs")).unwrap().bounds;
            app.click(logs.center());
            settle(&mut app);
            assert!(app.find(tr!("extensions.no_logs")).is_some());
        });
    }

    #[test]
    fn plugin_panels_render_open_and_route_commands() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        cairn_i18n::with_language(Language::ZhCn, || {
            let mut g = fixture(&rt, false);
            let entry = plugin(
                r#"{"id":"demo.panels","name":"Demo panels","version":"1.0.0","api":1,"main":"main.js",
                    "widgets":[{"id":"summary","name":"Summary","size":[4,2],"view":[{"element":"button","text":"Open News","action":{"type":"open_panel","panel":"news"}}]}],
                    "panels":[{"id":"news","title":"Demo News","icon":"globe","view":[{"element":"button","text":"Refresh News","action":{"type":"command","command":"refresh"}}]}]}"#,
            );
            let manifest = entry.manifest.clone().unwrap();
            g.m.plugins.entries = vec![entry];
            g.m.plugins.state.enable(&manifest);
            g.m.plugins.rebuild_widgets();
            assert_eq!(g.m.plugins.panels.len(), 1);

            let mut layout = cairn_bridge::BoardLayout::default();
            layout.items.push(cairn_bridge::BoardItem {
                id: "item-1".into(),
                kind: "plugin:demo.panels/summary".into(),
                x: 0,
                y: 0,
                w: 4,
                h: 2,
                options: Default::default(),
            });
            g.m.board = Some(layout);

            let theme = crate::style::theme(&g.m.appearance, None, false);
            let mut app = Headless::with_theme(g, Gui::view, 960.0, 600.0, 1.0, theme, text.take().unwrap());
            settle(&mut app);

            // Overview in settings shows panel list and "Open" button
            assert!(app.find(tr!("extensions.plugin_panels")).is_some());
            assert!(app.find("Demo News").is_some());
            let open_btn = app.find(tr!("extensions.open_panel")).expect("open panel button");
            app.click(open_btn.bounds.center());
            settle(&mut app);
            assert_eq!(app.state().ui.sheet, Some(Sheet::PluginPanel("demo.panels".into(), "news".into())));

            // Rendered plugin panel has title and button, command button triggers toast when not running
            assert!(app.find("Demo News").is_some());
            let cmd_btn = app.find("Refresh News").expect("panel command button");
            app.click(cmd_btn.bounds.center());
            settle(&mut app);
            assert!(app.state().m.toasts.iter().any(|t| t.text.contains("Demo panels")));

            // Missing plugin placeholder
            app.update(|g| g.open(Sheet::PluginPanel("missing.plugin".into(), "nope".into())));
            settle(&mut app);
            assert!(app.find(tr!("custom_panel.plugin_missing")).is_some());
            let manage_btn = app.find(tr!("extensions.manage")).expect("manage button");
            app.click(manage_btn.bounds.center());
            settle(&mut app);
            assert_eq!(app.state().ui.sheet, Some(Sheet::Settings));

            // Widget button with open_panel action from home dashboard
            app.update(|g| g.ui.sheet = None);
            settle(&mut app);
            let widget_btn = app.find("Open News").expect("widget open news button");
            app.click(widget_btn.bounds.center());
            settle(&mut app);
            assert_eq!(app.state().ui.sheet, Some(Sheet::PluginPanel("demo.panels".into(), "news".into())));
        });
    }
}
