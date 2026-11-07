//! Loader picker, shared by the install page and the instance modify page.
//! Each loader row expands to a version list. The install page also has an OptiFine row.
//! Which rows can be picked, and what auto picks, come from [`super::loader_plan`].

use cairn_bridge::support::{any_loader_supported, loader_supported};
use cairn_bridge::{LoaderKind, LoaderVersionInfo, ManifestEntry, Remote};
use cairn_i18n::tr;
use cairn_icons::VersionIcon;
use cairn_ui::{Align, El, Icon, Insets, Styled, checkbox, collapse, column, icon, icon_button, label, list_item, row, scroll, spinner, tag};

use super::controls::action_slot;
use super::loader_plan::{self, LOADERS, Plan, Row};
use crate::gui::{Gui, LoaderPick, VersionChoice};
use crate::style::{RED, fg};
use crate::widgets::version_tile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickTarget {
    Install,
    Modify,
}

/// Most versions a list shows. Beyond this only the total count is shown.
const LIST_LIMIT: usize = 120;

pub fn loader_icon(kind: LoaderKind) -> VersionIcon {
    match kind {
        LoaderKind::Fabric => VersionIcon::Fabric,
        LoaderKind::Quilt => VersionIcon::Quilt,
        LoaderKind::Forge => VersionIcon::Anvil,
        LoaderKind::NeoForge => VersionIcon::NeoForge,
        LoaderKind::OptiFine => VersionIcon::GrassPath,
        LoaderKind::LiteLoader => VersionIcon::Egg,
    }
}

pub fn loader_name(kind: LoaderKind) -> &'static str {
    match kind {
        LoaderKind::Fabric => "Fabric",
        LoaderKind::Quilt => "Quilt",
        LoaderKind::Forge => "Forge",
        LoaderKind::NeoForge => "NeoForge",
        LoaderKind::OptiFine => "OptiFine",
        LoaderKind::LiteLoader => "LiteLoader",
    }
}

impl Gui {
    pub fn pick(&self, t: PickTarget) -> &LoaderPick {
        match t {
            PickTarget::Install => &self.ui.install.pick,
            PickTarget::Modify => &self.ui.modify,
        }
    }

    fn pick_mut(&mut self, t: PickTarget) -> &mut LoaderPick {
        match t {
            PickTarget::Install => &mut self.ui.install.pick,
            PickTarget::Modify => &mut self.ui.modify,
        }
    }

    fn pick_mc(&self, t: PickTarget) -> Option<String> {
        match t {
            PickTarget::Install => self.ui.install.version.clone(),
            PickTarget::Modify => self.ui.version_id.as_deref().and_then(|id| self.m.instance(id)).and_then(|i| i.mc_version.clone()),
        }
    }

    /// Clicking a loader row only expands or collapses its version list. `None` confirms vanilla.
    fn click_loader(&mut self, t: PickTarget, kind: Option<LoaderKind>) {
        let p = self.pick_mut(t);
        if kind.is_some() {
            p.open = if p.open == kind { None } else { kind };
            return;
        }
        p.loader = None;
        p.loader_version = VersionChoice::Auto;
        p.open = None;
        self.after_pick(t);
    }

    /// Clicking the row only expands or collapses the version list.
    /// The cross at the row end clears a picked version.
    fn click_optifine(&mut self, t: PickTarget) {
        let p = self.pick_mut(t);
        p.open = if p.open == Some(LoaderKind::OptiFine) { None } else { Some(LoaderKind::OptiFine) };
    }

    fn clear(&mut self, t: PickTarget, kind: LoaderKind) {
        let p = self.pick_mut(t);
        if kind == LoaderKind::OptiFine {
            p.optifine = None;
        } else {
            p.loader = None;
            p.loader_version = VersionChoice::Auto;
        }
        if p.open == Some(kind) {
            p.open = None;
        }
        self.after_pick(t);
    }

    /// The OptiFine list only shows builds that pair with the chosen Forge.
    /// When the Forge version changes, a picked OptiFine that no longer pairs falls back to auto.
    fn click_version(&mut self, t: PickTarget, kind: LoaderKind, choice: VersionChoice) {
        let mc = self.pick_mc(t).unwrap_or_default();
        let stale_optifine = match (kind, &choice, self.pick(t).optifine.as_ref().and_then(VersionChoice::exact)) {
            (LoaderKind::Forge, VersionChoice::Exact(forge), Some(of)) => match self.m.loaders.get(&mc, LoaderKind::OptiFine) {
                Some(Remote::Ready(list)) => list.iter().find(|v| v.version == of).is_none_or(|v| !loader_plan::forge_fits(&mc, v, forge)),
                _ => false,
            },
            _ => false,
        };
        let p = self.pick_mut(t);
        if kind == LoaderKind::OptiFine {
            p.optifine = Some(choice);
        } else {
            p.loader = Some(kind);
            p.loader_version = choice;
            if loader_plan::kind_conflict(&mc, kind).is_some() {
                p.optifine = None;
            } else if stale_optifine {
                p.optifine = Some(VersionChoice::Auto);
            }
        }
        p.open = None;
        self.after_pick(t);
    }

    /// Runs after a pick. On the install page, refreshes the default instance name.
    fn after_pick(&mut self, t: PickTarget) {
        if t == PickTarget::Install {
            self.refresh_default_name();
        }
    }
}

/// The manifest entry for `mc`. `None` until the manifest has loaded.
pub fn mc_entry<'a>(g: &'a Gui, mc: &str) -> Option<&'a ManifestEntry> {
    g.m.manifest.ready()?.iter().find(|e| e.id == mc)
}

/// Whether this loader can be installed on `mc`. Treated as supported until the manifest loads,
/// so unsupported rows are only hidden once it arrives.
pub fn supported(g: &Gui, kind: LoaderKind, mc: &str) -> bool {
    mc_entry(g, mc).is_none_or(|e| loader_supported(kind, e))
}

pub fn plan_for<'a>(g: &'a Gui, t: PickTarget, mc: &str) -> Plan<'a> {
    loader_plan::plan(mc, g.pick(t), |k| g.m.loaders.get(mc, k))
}

/// Lists the loader rows for `mc`: the main loader first, then the add-on rows when `with_addons` is set.
/// Loaders that `mc` does not support are hidden, so very old versions only show vanilla.
pub fn picker(g: &Gui, t: PickTarget, mc: &str, with_addons: bool) -> El<Gui> {
    let p = g.pick(t);
    let plan = plan_for(g, t, mc);
    let group = |kind: LoaderKind, selected: bool, chosen: Option<&VersionChoice>| -> El<Gui> {
        let row = plan.row(kind);
        let is_open = p.open == Some(kind) && matches!(row, Some(Row::Ready { .. }));
        let list: El<Gui> = match row {
            Some(Row::Ready { versions, auto }) => version_list(t, kind, versions, *auto, chosen),
            _ => column().into(),
        };
        column()
            .key(("loader_group", kind as u8))
            .fill_width()
            .align(Align::Stretch)
            .child(loader_row(t, kind, mc, row, selected, chosen, is_open))
            .child(collapse(is_open).key(("loader_collapse", kind as u8)).fill_width().child(list))
            .into()
    };

    let mut col = column().fill_width().gap(3.0).align(Align::Stretch);
    col = col.child(
        list_item()
            .key("vanilla")
            .padding(Insets::symmetric(10.0, 6.0))
            .gap(10.0)
            .selected(p.loader.is_none())
            .on_click(move |g: &mut Gui| g.click_loader(t, None))
            .child(version_tile::<Gui>(VersionIcon::Grass, 30.0))
            .child(
                column()
                    .grow(1.0)
                    .min_width(0.0)
                    .gap(1.0)
                    .child(label(tr!("loaders.vanilla")).weight(700).color(fg(0.92)))
                    .child(label(tr!("loaders.cannot_install_mods")).caption()),
            ),
    );
    if mc_entry(g, mc).is_some_and(|e| !any_loader_supported(e)) {
        col = col.child(label(tr!("loaders.version_too_old")).caption().margin(Insets::new(4.0, 0.0, 0.0, 10.0)));
    }
    for kind in LOADERS.into_iter().filter(|k| supported(g, *k, mc)) {
        let selected = p.loader == Some(kind);
        col = col.child(group(kind, selected, selected.then_some(&p.loader_version)));
    }
    if !with_addons || mc.is_empty() {
        return col.into();
    }

    let mut addons: Vec<El<Gui>> = Vec::new();
    if let Some(k @ (LoaderKind::Fabric | LoaderKind::Quilt)) = p.loader {
        let name = if k == LoaderKind::Quilt { "Quilt Standard Libraries" } else { "Fabric API" };
        addons.push(
            row()
                .key("api_mod")
                .padding(Insets::new(2.0, 10.0, 2.0, 10.0))
                .child(checkbox(p.api_mod).label(name).on_change(move |g: &mut Gui, on| g.pick_mut(t).api_mod = on))
                .into(),
        );
    }
    if supported(g, LoaderKind::OptiFine, mc) {
        addons.push(group(LoaderKind::OptiFine, p.optifine.is_some(), p.optifine.as_ref()));
        if p.loader == Some(LoaderKind::Fabric) && p.optifine.is_some() {
            addons.push(label(tr!("loaders.optifabric_hint")).caption().margin(Insets::new(0.0, 10.0, 0.0, 10.0)).into());
        }
    }
    if !addons.is_empty() {
        col = col.child(label(tr!("loaders.addons_section")).secondary().margin(Insets::new(8.0, 0.0, 2.0, 0.0))).children(addons);
    }
    col.into()
}

fn loader_row(t: PickTarget, kind: LoaderKind, mc: &str, state: Option<&Row>, selected: bool, chosen: Option<&VersionChoice>, open: bool) -> El<Gui> {
    let caption = |s: String| -> El<Gui> { label(s).caption().truncate().height(18.0).into() };
    let (detail, usable, retry): (El<Gui>, bool, bool) = match state {
        _ if mc.is_empty() => (caption(tr!("loaders.select_game_version_first").into()), false, false),
        Some(Row::Ready { auto, .. }) => {
            let text = match (chosen, auto) {
                (Some(VersionChoice::Exact(v)), _) => v.clone(),
                (_, Some(_)) if !selected => tr!("loaders.select_version").into(),
                (_, Some(a)) => format!("{} · {}", tr!("settings.gc_auto"), a.version),
                (_, None) => tr!("loaders.no_compatible_version").into(),
            };
            (caption(text), true, false)
        }
        Some(Row::Blocked(reason)) => (caption(reason.clone()), false, false),
        Some(Row::Unsupported) => (caption(tr!("loaders.unsupported_game_version").into()), false, false),
        Some(Row::Failed) => (label(tr!("loaders.fetch_failed_retry")).caption().truncate().height(18.0).color(RED).into(), false, true),
        Some(Row::Loading) | None => (row().height(18.0).gap(6.0).child(spinner()).child(label(tr!("loaders.fetching")).caption()).into(), false, false),
    };
    let mut r = list_item()
        .key(("loader", kind as u8))
        .padding(Insets::symmetric(10.0, 6.0))
        .gap(10.0)
        .selected(selected)
        .child(version_tile::<Gui>(loader_icon(kind), 30.0).muted(!usable))
        .child(
            column().grow(1.0).min_width(0.0).gap(1.0).child(label(loader_name(kind)).weight(700).color(if usable { fg(0.92) } else { fg(0.4) })).child(detail),
        );
    if usable {
        r = if kind == LoaderKind::OptiFine {
            r.on_click(move |g: &mut Gui| g.click_optifine(t))
        } else {
            r.on_click(move |g: &mut Gui| g.click_loader(t, Some(kind)))
        };
    } else if retry {
        let mc = mc.to_string();
        r = r.on_click(move |g: &mut Gui| g.bridge.retry_loader(&mut g.m, &mc, kind));
    }
    let indicator = if usable { Some(if open { Icon::ChevronUp } else { Icon::ChevronDown }) } else { retry.then_some(Icon::Refresh) };
    r.child(row().gap(6.0).child(action_slot(18.0, indicator.map(|ic| icon(ic).icon_size(14.0).into()))).child(action_slot(
        32.0,
        selected.then(|| icon_button(Icon::Close).label(tr!("loaders.do_not_install")).on_click(move |g: &mut Gui| g.clear(t, kind)).into()),
    )))
    .into()
}

/// The expanded list: 'auto' first, then the versions that fit the other choices.
fn version_list(t: PickTarget, kind: LoaderKind, versions: &[&LoaderVersionInfo], auto: Option<&LoaderVersionInfo>, chosen: Option<&VersionChoice>) -> El<Gui> {
    let auto_text = match auto {
        Some(b) => format!("{} ({} {})", tr!("settings.gc_auto"), tr!("loaders.recommended"), b.version),
        None => tr!("settings.gc_auto").to_string(),
    };
    let exact = chosen.and_then(VersionChoice::exact);
    let mut rows: Vec<El<Gui>> = vec![
        list_item()
            .key("auto")
            .padding(Insets::symmetric(10.0, 5.0))
            .selected(matches!(chosen, Some(VersionChoice::Auto)))
            .on_click(move |g: &mut Gui| g.click_version(t, kind, VersionChoice::Auto))
            .child(label(auto_text).color(fg(0.92)).grow(1.0))
            .into(),
    ];
    rows.extend(versions.iter().take(LIST_LIMIT).map(|v| {
        let version = v.version.clone();
        let mut r = list_item()
            .key(v.version.clone())
            .padding(Insets::symmetric(10.0, 5.0))
            .selected(exact == Some(v.version.as_str()))
            .on_click(move |g: &mut Gui| g.click_version(t, kind, VersionChoice::Exact(version.clone())))
            .child(label(&v.version).color(fg(0.92)).truncate().grow(1.0));
        if v.recommended {
            r = r.child(tag(tr!("loaders.recommended")).accent());
        } else if !v.stable {
            r = r.child(tag(tr!("loaders.beta")));
        }
        if let Some(req) = v.requires.as_deref().filter(|s| !s.trim().is_empty()) {
            r = r.child(label(req).caption());
        }
        r.into()
    }));
    if versions.len() > LIST_LIMIT {
        rows.push(label(tr!("loaders.more_versions_hidden", count = versions.len() - LIST_LIMIT)).caption().margin(Insets::new(4.0, 10.0, 2.0, 10.0)).into());
    }
    // The list fills the row width. Indent it with the outer padding, which counts toward the width.
    // Adding an outer margin on top of full width would overflow the row and clip on the right.
    let card = column()
        .fill_width()
        .align(Align::Stretch)
        .padding(4.0)
        .background(fg(0.045))
        .corner_radius(10.0)
        .child(scroll().fill_width().max_height(200.0).align(Align::Stretch).gap(1.0).children(rows));
    column().fill_width().align(Align::Stretch).padding(Insets::new(2.0, 0.0, 6.0, 40.0)).child(card).into()
}

/// Icon for the instance list and dock. A picked icon wins, then a loader icon if one is installed.
/// Vanilla uses the version type: release grass block, snapshot command block, old cobblestone,
/// April Fools gold block; errors show a redstone block.
pub fn instance_icon(m: &cairn_bridge::Model, inst: &cairn_bridge::Instance) -> VersionIcon {
    version_icon(m, inst.mc_version.as_deref().unwrap_or(&inst.version_id), &inst.loaders, inst.settings.icon.as_deref(), inst.problem.is_some())
}

pub(super) fn version_icon(m: &cairn_bridge::Model, id: &str, loaders: &[cairn_bridge::LoaderInfo], chosen: Option<&str>, broken: bool) -> VersionIcon {
    if let Some(chosen) = chosen.and_then(VersionIcon::from_key) {
        return chosen;
    }
    if broken {
        return VersionIcon::RedstoneBlock;
    }
    for kind in [LoaderKind::Fabric, LoaderKind::Quilt, LoaderKind::Forge, LoaderKind::NeoForge, LoaderKind::LiteLoader, LoaderKind::OptiFine] {
        if loaders.iter().any(|l| l.kind == kind) {
            return loader_icon(kind);
        }
    }
    let kind = m.manifest.ready().and_then(|all| all.iter().find(|e| e.id == id)).map(|e| e.kind);
    match version_class(id, kind) {
        VersionClass::Fool => VersionIcon::GoldBlock,
        VersionClass::Snapshot => VersionIcon::CommandBlock,
        VersionClass::Old => VersionIcon::Cobblestone,
        VersionClass::Release => VersionIcon::Grass,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionClass {
    Release,
    Snapshot,
    Old,
    Fool,
}

/// The category of a game version. Uses the manifest type when listed. Otherwise (manifest not
/// loaded, or a locally renamed version) it guesses from the id.
pub fn version_class(id: &str, listed: Option<cairn_bridge::VersionType>) -> VersionClass {
    use cairn_bridge::VersionType;
    if cairn_bridge::support::is_fool_version(id) {
        return VersionClass::Fool;
    }
    match listed {
        Some(VersionType::Snapshot) => return VersionClass::Snapshot,
        Some(VersionType::OldAlpha | VersionType::OldBeta) => return VersionClass::Old,
        Some(_) => return VersionClass::Release,
        None => {}
    }
    let lower = id.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let old = (matches!(bytes.first(), Some(b'a' | b'b')) && bytes.get(1).is_some_and(u8::is_ascii_digit))
        || ["rd-", "c0.", "inf-"].iter().any(|p| lower.starts_with(p));
    if old {
        return VersionClass::Old;
    }
    let weekly = bytes.len() >= 6
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'w'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit();
    if weekly || lower.contains("-pre") || lower.contains("-rc") || lower.contains("snapshot") {
        return VersionClass::Snapshot;
    }
    VersionClass::Release
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_bridge::{Bridge, Sink, VersionType};
    use cairn_ui::testing::Headless;
    use std::sync::Arc;

    fn gui(rt: &tokio::runtime::Runtime) -> Gui {
        let sink: Sink = Arc::new(|_| true);
        let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        g.ui.install.version = Some("1.20.1".into());
        g.refresh_default_name();
        g
    }

    #[test]
    fn loader_actions_and_following_rows_do_not_move_between_states() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for language in [cairn_bridge::Language::ZhCn, cairn_bridge::Language::EnUs, cairn_bridge::Language::JaJp] {
            cairn_i18n::with_language(language, || {
                let view = |g: &Gui| {
                    let version = LoaderVersionInfo { version: "0.20.1".into(), stable: true, recommended: true, requires: None };
                    let state = match g.ui.offline_name.as_str() {
                        "loading" => Row::Loading,
                        "failed" => Row::Failed,
                        "blocked" => Row::Blocked("Unavailable".into()),
                        _ => Row::Ready { versions: vec![&version], auto: Some(&version) },
                    };
                    let p = g.pick(PickTarget::Install);
                    column()
                        .width(360.0)
                        .align(Align::Stretch)
                        .child(loader_row(
                            PickTarget::Install,
                            LoaderKind::Fabric,
                            "1.20.1",
                            Some(&state),
                            p.loader.is_some(),
                            p.loader.as_ref().map(|_| &p.loader_version),
                            p.open.is_some(),
                        ))
                        .child(label("following row"))
                };
                let theme = crate::style::theme(&cairn_bridge::Appearance::default(), None, false);
                let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
                let mut app = Headless::with_theme(gui(&rt), view, 400.0, 180.0, 1.5, theme, text);
                let following = app.find("following row").unwrap().bounds;
                let bounds = app.find_role(cairn_ui::Role::ListItem)[0].bounds;
                let arrow = |app: &Headless<Gui>| app.nodes().into_iter().find(|node| node.bounds.width == 14.0 && node.bounds.height == 14.0).unwrap().bounds;
                let arrow_bounds = arrow(&app);
                app.update(|g| {
                    g.ui.install.pick.loader = Some(LoaderKind::Fabric);
                    g.ui.install.pick.open = Some(LoaderKind::Fabric);
                });
                assert_eq!(arrow(&app), arrow_bounds);
                let clear = app.find(tr!("loaders.do_not_install")).unwrap().bounds;
                assert!(clear.x > arrow_bounds.right());
                for state in ["loading", "failed", "blocked", "ready"] {
                    app.update(move |g| g.ui.offline_name = state.into());
                    assert_eq!(app.find_role(cairn_ui::Role::ListItem)[0].bounds, bounds, "{state}");
                    assert_eq!(app.find("following row").unwrap().bounds, following, "{state}");
                    assert_eq!(app.find(tr!("loaders.do_not_install")).unwrap().bounds, clear, "{state}");
                }
                app.click(clear.center());
                assert!(app.state().ui.install.pick.loader.is_none());
            });
        }
    }

    #[test]
    fn browsing_loaders_preserves_the_confirmed_selection() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for t in [PickTarget::Install, PickTarget::Modify] {
            let mut g = gui(&rt);
            let name = g.ui.install.name.clone();
            g.click_loader(t, Some(LoaderKind::Fabric));
            assert_eq!(g.pick(t).loader, None);
            assert_eq!(g.pick(t).open, Some(LoaderKind::Fabric));
            assert_eq!(g.ui.install.name, name);
            assert!(plan_for(&g, t, "1.20.1").spec().is_none());
            g.click_loader(t, Some(LoaderKind::Quilt));
            assert_eq!(g.pick(t).loader, None);
            assert_eq!(g.pick(t).open, Some(LoaderKind::Quilt));

            g.click_version(t, LoaderKind::Fabric, VersionChoice::Exact("0.20.1".into()));
            assert_eq!(g.pick(t).loader, Some(LoaderKind::Fabric));
            assert_eq!(g.pick(t).open, None);
            let selected = g.pick(t).clone();
            let name = g.ui.install.name.clone();
            g.click_loader(t, Some(LoaderKind::Quilt));
            assert_eq!(g.pick(t).loader, selected.loader);
            assert_eq!(g.pick(t).loader_version, selected.loader_version);
            assert_eq!(g.ui.install.name, name);
            g.click_loader(t, Some(LoaderKind::Quilt));
            assert_eq!(g.pick(t), &selected);
            g.click_loader(t, None);
            assert_eq!(g.pick(t).loader, None);
            assert_eq!(g.pick(t).open, None);
        }
    }

    #[test]
    fn addons_and_conflicts_change_only_after_choosing_a_version() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = gui(&rt);
        g.click_optifine(PickTarget::Install);
        assert!(g.ui.install.pick.optifine.is_none());
        assert_eq!(g.ui.install.pick.open, Some(LoaderKind::OptiFine));
        g.click_version(PickTarget::Install, LoaderKind::OptiFine, VersionChoice::Auto);
        assert_eq!(g.ui.install.pick.optifine, Some(VersionChoice::Auto));
        let name = g.ui.install.name.clone();
        g.click_loader(PickTarget::Install, Some(LoaderKind::NeoForge));
        assert_eq!(g.ui.install.pick.optifine, Some(VersionChoice::Auto));
        assert_eq!(g.ui.install.name, name);
        g.click_version(PickTarget::Install, LoaderKind::NeoForge, VersionChoice::Exact("47.1.106".into()));
        assert!(g.ui.install.pick.optifine.is_none());
        assert_eq!(g.ui.install.pick.loader, Some(LoaderKind::NeoForge));
        g.clear(PickTarget::Install, LoaderKind::NeoForge);
        assert_eq!(g.ui.install.pick.loader, None);
        assert_eq!(g.ui.install.name, "1.20.1");
    }

    #[test]
    fn version_rows_are_unselected_until_the_user_clicks_one() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(cairn_bridge::Language::ZhCn, || {
            let view = |g: &Gui| {
                let v = LoaderVersionInfo { version: "0.20.1".into(), stable: true, recommended: true, requires: None };
                let p = g.pick(PickTarget::Install);
                version_list(PickTarget::Install, LoaderKind::Fabric, &[&v], Some(&v), (p.loader == Some(LoaderKind::Fabric)).then_some(&p.loader_version))
            };
            let mut app = Headless::new(gui(&rt), view, 440.0, 240.0, 1.0);
            app.render();
            assert!(app.find_role(cairn_ui::Role::ListItem).iter().all(|n| n.semantics.value.is_none()));
            assert!(app.find(tr!("loaders.installed")).is_none());
            let auto = app.find("自动 (推荐 0.20.1)").unwrap();
            app.click(auto.bounds.center());
            assert_eq!(app.state().ui.install.pick.loader, Some(LoaderKind::Fabric));
            assert_eq!(app.state().ui.install.pick.loader_version, VersionChoice::Auto);
            assert_eq!(app.find_role(cairn_ui::Role::ListItem).iter().filter(|n| n.semantics.value.is_some()).count(), 1);
            let exact = app.find("0.20.1").unwrap();
            app.click(exact.bounds.center());
            assert_eq!(app.state().ui.install.pick.loader_version, VersionChoice::Exact("0.20.1".into()));
        });
    }

    #[test]
    fn version_list_fits_inside_its_row() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let versions: Vec<LoaderVersionInfo> =
            (0..12).map(|i| LoaderVersionInfo { version: format!("0.{}.0", 20 - i), stable: i == 0, recommended: i == 0, requires: None }).collect();
        let view = move |_: &Gui| {
            let refs: Vec<&LoaderVersionInfo> = versions.iter().collect();
            column().padding(20.0).child(column().width(320.0).align(Align::Stretch).child(version_list(
                PickTarget::Install,
                LoaderKind::Fabric,
                &refs,
                refs.first().copied(),
                None,
            )))
        };
        let mut app = Headless::new(gui(&rt), view, 400.0, 300.0, 1.0);
        app.render();
        let rows = app.find_role(cairn_ui::Role::ListItem);
        assert!(!rows.is_empty());
        for r in &rows {
            // Left edge sits under the loader name. The right edge stays inside the row, scrollbar included.
            assert!(r.bounds.x >= 20.0 + 40.0, "{:?}", r.bounds);
            assert!(r.bounds.right() <= 20.0 + 320.0 - 14.0, "{:?}", r.bounds);
        }
        let w = rows[0].bounds.width;
        assert!(rows.iter().all(|r| (r.bounds.width - w).abs() < 0.5) && w > 240.0, "{w}");
    }

    #[test]
    fn version_class_uses_the_manifest_when_it_knows_the_version() {
        assert_eq!(version_class("1.21.5", Some(VersionType::Release)), VersionClass::Release);
        assert_eq!(version_class("25w42a", Some(VersionType::Snapshot)), VersionClass::Snapshot);
        assert_eq!(version_class("b1.7.3", Some(VersionType::OldBeta)), VersionClass::Old);
        assert_eq!(version_class("a1.2.6", Some(VersionType::OldAlpha)), VersionClass::Old);
        // April Fools versions are snapshots in the manifest, so they must be recognized by id first.
        assert_eq!(version_class("24w14potato", Some(VersionType::Snapshot)), VersionClass::Fool);
    }

    #[test]
    fn version_class_guesses_from_the_id_without_a_manifest() {
        for id in ["1.21.5", "26.3", "1.7.10", "1.0"] {
            assert_eq!(version_class(id, None), VersionClass::Release, "{id}");
        }
        for id in ["24w14a", "1.21-pre1", "1.21-rc2", "1.20.5-snapshot-1"] {
            assert_eq!(version_class(id, None), VersionClass::Snapshot, "{id}");
        }
        for id in ["b1.7.3", "a1.2.6", "rd-132211", "c0.0.13a", "inf-20100618"] {
            assert_eq!(version_class(id, None), VersionClass::Old, "{id}");
        }
        assert_eq!(version_class("3D Shareware v1.34", None), VersionClass::Fool);
    }
}
