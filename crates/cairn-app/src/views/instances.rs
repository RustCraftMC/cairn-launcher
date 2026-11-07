//! Instances page: game folders on the left, the instances of the selected folder on the right.
//! It also hosts the install flow for new versions.

use cairn_i18n::tr;

use cairn_bridge::{
    Folder, FolderKind, InstallRequest, InstallTarget, Instance, LoaderKind, ManifestEntry, OptiFineChoice, Remote, VersionType,
};
use cairn_ui::{Align, El, Icon, Insets, Styled, button, column, divider, icon, icon_button, label, list_item, row, scroll, spinner, tab, tag, text_input};

use super::controls::action_slot;
use super::dock::describe;
use super::format;
use super::loader_plan;
use super::loaders::{self, PickTarget, loader_name, picker};
use super::{SCROLL_GUTTER, empty_state, frame, header, loading_row, page_reveal, page_switch, removable};
use crate::gui::{Confirm, Gui, InstallForm, LoaderPick, Removal, Sheet, VersionChoice, VersionFilter};
use crate::style::{RED, fg};
use crate::views::loaders::instance_icon;
use crate::widgets::version_tile;

pub fn list(g: &Gui) -> El<Gui> {
    let folders = g.m.folders();
    let current = g.current_folder();
    let folder = folders.iter().find(|f| f.name == current).or(folders.first()).cloned();
    let current = folder.as_ref().map(|f| f.name.clone()).unwrap_or(current);
    let count = g.m.folder_instances(&current).count();
    // The title never wraps. With long Japanese button labels, the count is truncated first.
    // This keeps the title from being squeezed into vertical text.
    let lead = row()
        .gap(10.0)
        .min_width(0.0)
        .child(label(tr!("instances.title")).title().shrink(0.0))
        .child(label(tr!("instances.instances_count", count = count)).caption().truncate().min_width(0.0));
    let actions = vec![
        button(tr!("pack.drop_pack")).icon(Icon::Download).on_click(|g: &mut Gui| g.bridge.pick_pack(&mut g.m)).into(),
        button(tr!("instances.new")).icon(Icon::Plus).accent().on_click(|g: &mut Gui| g.open_new_instance()).into(),
    ];
    let body = row()
        .grow(1.0)
        .min_height(0.0)
        .fill_width()
        .align(Align::Stretch)
        .child(folder_column(g, &folders, &current))
        .child(divider().vertical())
        .child(instance_column(g, folder.as_ref()));
    frame(header(lead, actions), body.into(), None)
}

pub fn folder_icon(kind: FolderKind) -> Icon {
    match kind {
        FolderKind::Current => Icon::Home,
        FolderKind::Official => Icon::Cube,
        FolderKind::Custom => Icon::Folder,
    }
}

fn folder_column(g: &Gui, folders: &[Folder], current: &str) -> El<Gui> {
    let rows = folders.iter().map(|f| {
        let name = f.name.clone();
        let n = g.m.folder_instances(&f.name).count();
        let detail = f.path.display().to_string();
        list_item()
            .key(("folder", f.name.clone()))
            .padding(Insets::symmetric(10.0, 7.0))
            .gap(10.0)
            .selected(f.name == current)
            .on_click(move |g: &mut Gui| g.ui.folder = Some(name.clone()))
            .child(icon(folder_icon(f.kind)).icon_size(18.0))
            .child(
                column()
                    .grow(1.0)
                    .min_width(0.0)
                    .gap(1.0)
                    .child(label(f.label()).weight(700).color(fg(0.92)).truncate())
                    .child(label(detail).caption().truncate()),
            )
            .child(label(n.to_string()).caption().shrink(0.0))
    });
    column()
        .width(240.0)
        .shrink(0.0)
        .align(Align::Stretch)
        .gap(2.0)
        .padding(Insets::new(10.0, 10.0, 12.0, 10.0))
        .child(label(tr!("instances.minecraft_folder")).secondary().margin(Insets::new(0.0, 0.0, 4.0, 6.0)))
        .child(scroll().grow(1.0).min_height(0.0).align(Align::Stretch).gap(2.0).children(rows))
        .child(button(tr!("instances.add_game_dir")).icon(Icon::Plus).on_click(|g: &mut Gui| g.bridge.pick_folder(&mut g.m)))
        .into()
}

fn instance_column(g: &Gui, folder: Option<&Folder>) -> El<Gui> {
    let Some(folder) = folder else {
        return column().grow(1.0).child(empty_state(Icon::Folder, tr!("instances.no_dirs"), tr!("instances.no_dirs_desc"))).into();
    };
    let path = folder.path.clone();
    let name = folder.name.clone();
    let mut info = row().fill_width().gap(8.0).padding(Insets::new(10.0, 14.0, 4.0, 16.0)).child(
        column()
            .grow(1.0)
            .min_width(0.0)
            .gap(1.0)
            .child(label(folder.label()).weight(700).color(fg(0.95)))
            .child(label(folder.path.display().to_string()).caption().truncate()),
    );
    info = info.child(button(tr!("common.open")).icon(Icon::ExternalLink).on_click(move |g: &mut Gui| {
        let dir = path.clone();
        g.bridge.open_path(&dir);
    }));
    if folder.removable() {
        if g.ui.confirm == Some(Confirm::Folder(name.clone())) {
            let n = name.clone();
            info = info
                .child(label(tr!("instances.remove_dir_safe_tip")).font_size(13.0).color(RED))
                .child(button(tr!("common.delete")).on_click(move |g: &mut Gui| {
                    g.ui.confirm = None;
                    g.ui.folder = None;
                    g.bridge.remove_folder(&mut g.m, &n);
                }))
                .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.ui.confirm = None));
        } else {
            info = info.child(
                icon_button(Icon::Trash).label(tr!("instances.remove_dir")).on_click(move |g: &mut Gui| g.ui.confirm = Some(Confirm::Folder(name.clone()))),
            );
        }
    }
    let body: El<Gui> = match &g.m.instances {
        Remote::Ready(_) => {
            let mut items: Vec<&Instance> = g.m.folder_instances(&folder.name).collect();
            // Favorites go first. The sort is stable, so the other instances keep their order.
            items.sort_by_key(|i| !i.settings.favorite);
            if items.is_empty() {
                let target = folder.name.clone();
                column()
                    .grow(1.0)
                    .fill_width()
                    .align(Align::Center)
                    .child(empty_state(Icon::Gamepad, tr!("instances.empty"), tr!("instances.empty_desc")))
                    .child(button(tr!("common.install")).accent().icon(Icon::Download).margin(Insets::new(0.0, 0.0, 36.0, 0.0)).on_click(move |g: &mut Gui| {
                        g.ui.install.folder = Some(target.clone());
                        g.open_new_instance();
                    }))
                    .into()
            } else {
                scroll()
                    .grow(1.0)
                    .align(Align::Stretch)
                    .padding(Insets::new(4.0, 12.0, 12.0, 12.0))
                    .gap(2.0)
                    .children(items.into_iter().map(|i| removable(g, Removal::Instance(i.id.clone()), instance_row(g, i))))
                    .into()
            }
        }
        Remote::Failed(e) => empty_state(Icon::Warning, &tr!("instances.read_failed", error = e), ""),
        _ => column().grow(1.0).child(loading_row(tr!("instances.reading"))).into(),
    };
    let col = column().grow(1.0).min_width(0.0).min_height(0.0).align(Align::Stretch).child(info).child(body);
    page_reveal(("instance_folder", &folder.name), col)
}

fn instance_row(g: &Gui, i: &Instance) -> El<Gui> {
    let id = i.id.clone();
    let selected = g.m.selected.as_deref() == Some(i.id.as_str());
    let layout = if i.game_dir == i.root { tr!("instances.shared_dir") } else { tr!("settings.isolation_mods") };
    let mut r = list_item()
        .key(i.id.clone())
        .selected(selected)
        .on_click({
            let id = id.clone();
            move |g: &mut Gui| {
                g.bridge.select_instance(&mut g.m, &id);
                g.close();
            }
        })
        .on_context_menu({
            let id = id.clone();
            move |g: &mut Gui, at| g.open_instance_menu(&id, at)
        })
        .child(version_tile::<Gui>(instance_icon(&g.m, i), 40.0))
        .child(
            column()
                .grow(1.0)
                .min_width(0.0)
                .gap(2.0)
                .child(label(&i.name).truncate().weight(700).color(fg(0.95)))
                .child(label(format!("{} · {layout}", describe(i))).caption().truncate()),
        );
    if i.settings.favorite {
        r = r.child(tag(tr!("instances.favorite")).accent());
    }
    if i.problem.is_some() {
        r = r.child(tag(tr!("instances.broken_files")));
    }
    let action: El<Gui> = if g.m.game(&id).is_some() {
        tag(tr!("tasks.game_running")).accent().into()
    } else if g.m.launch_task(&id).is_some() {
        spinner().into()
    } else {
        let launch = id.clone();
        button(tr!("dock.launch")).icon(Icon::Play).on_click(move |g: &mut Gui| g.launch_instance(&launch)).into()
    };
    r = r.child(action_slot(136.0, Some(action)));
    r.child(icon_button(Icon::Gear).label(tr!("instance_menu.instance_settings")).on_click(move |g: &mut Gui| g.open_version(&id))).into()
}

/// Default instance name on the install page: `version-loader`, plus `-OptiFine` if used.
fn default_name(version: &str, pick: &LoaderPick) -> String {
    let mut name = version.to_string();
    if let Some(k) = pick.loader {
        name = format!("{name}-{}", loader_name(k));
    }
    if pick.optifine.is_some() {
        name.push_str("-OptiFine");
    }
    name
}

/// Characters a version name cannot contain, following the Windows file name rules.
fn bad_name_char(c: char) -> bool {
    matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
}

impl Gui {
    pub fn open_new_instance(&mut self) {
        self.open(Sheet::NewInstance);
    }

    /// Game folder to install into, by registered name.
    /// An empty string means the standalone instance library.
    pub fn install_folder(&self) -> String {
        self.ui.install.folder.clone().unwrap_or_else(|| self.current_folder())
    }

    /// A version was picked. The version list fades out, then the loader step shows.
    /// Loader versions are fetched right away.
    fn choose_version(&mut self, v: String) {
        self.bridge.load_loaders(&mut self.m, &v);
        self.ui.install.next_step = Some(Some(v));
    }

    fn arrive_install_step(&mut self) {
        match self.ui.install.next_step.take() {
            Some(Some(v)) => self.pick_version(v),
            Some(None) => self.back_to_versions(),
            None => {}
        }
    }

    /// Goes back from the loader step to the version step. The loader choice is kept.
    /// An incompatible loader is cleared when the version changes.
    fn back_to_versions(&mut self) {
        self.ui.install.version = None;
        self.ui.install.pick.open = None;
    }

    pub fn pick_version(&mut self, v: String) {
        let f = &mut self.ui.install;
        // A loader version belongs to the old game version.
        // Changing the game version therefore resets it to Auto.
        f.pick.loader_version = VersionChoice::Auto;
        if f.pick.optifine.is_some() {
            f.pick.optifine = Some(VersionChoice::Auto);
        }
        f.pick.open = None;
        f.version = Some(v.clone());
        // If the new version does not support the chosen loader (for example 1.20 to 1.12),
        // fall back to vanilla.
        if self.ui.install.pick.loader.is_some_and(|k| !loaders::supported(self, k, &v)) {
            self.ui.install.pick.loader = None;
        }
        let p = &self.ui.install.pick;
        if p.optifine.is_some()
            && (!loaders::supported(self, LoaderKind::OptiFine, &v) || p.loader.is_some_and(|k| loader_plan::kind_conflict(&v, k).is_some()))
        {
            self.ui.install.pick.optifine = None;
        }
        self.refresh_default_name();
        self.bridge.load_loaders(&mut self.m, &v);
    }

    /// Regenerates the default name from the current choices, unless the user has edited the name.
    pub fn refresh_default_name(&mut self) {
        let f = &mut self.ui.install;
        if !f.name_edited
            && let Some(v) = &f.version
        {
            f.name = default_name(v, &f.pick);
        }
    }

    fn name_problem(&self) -> Option<String> {
        self.folder_name_problem(&self.install_folder(), &self.ui.install.name)
    }

    /// Why `name` cannot be used as a version name in a game folder, or None. In a standard folder
    /// the version name is the folder name, so it must be unique and free of illegal characters.
    /// An empty name is handled elsewhere. The bridge makes names unique in the standalone library.
    pub fn folder_name_problem(&self, folder: &str, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() || folder.is_empty() {
            return None;
        }
        if name.chars().any(bad_name_char) {
            return Some(tr!("instances.invalid_chars").into());
        }
        if name.ends_with('.') {
            return Some(tr!("instances.trailing_dot").into());
        }
        let id = format!("{folder}/{name}");
        self.m.instances.ready().is_some_and(|l| l.iter().any(|i| i.id.eq_ignore_ascii_case(&id))).then(|| tr!("instances.name_exists").to_string())
    }

    /// Why the install cannot start now. `None` means it can.
    fn install_problem(&self) -> Option<String> {
        let f = &self.ui.install;
        let Some(mc) = f.version.as_deref() else { return Some(tr!("instances.select_version_left").into()) };
        if f.name.trim().is_empty() {
            return Some(tr!("instances.enter_name").into());
        }
        if let Some(problem) = self.name_problem() {
            return Some(problem);
        }
        loaders::plan_for(self, PickTarget::Install, mc).problem
    }

    fn start_install(&mut self) {
        if self.install_problem().is_some() {
            return;
        }
        let f = &self.ui.install;
        let mc = f.version.clone().unwrap_or_default();
        let plan = loaders::plan_for(self, PickTarget::Install, &mc);
        let loader = plan.spec();
        // Submit the version shown on screen, so the core does not pick another one by its own rules.
        let optifine = plan.optifine.clone().map(|v| OptiFineChoice { version: Some(v) });
        let api_mod = f.pick.api_mod && matches!(f.pick.loader, Some(LoaderKind::Fabric | LoaderKind::Quilt));
        let name = f.name.trim().to_string();
        let folder = self.install_folder();
        let target = InstallTarget::Standard { root: folder.clone(), version_name: Some(name) };
        let req = InstallRequest { mc_version: mc, loader, optifine, fabric_api: api_mod, target };
        self.bridge.install(&mut self.m, req);
        let filter = self.ui.install.filter;
        self.ui.install = InstallForm { filter, folder: Some(folder), ..InstallForm::default() };
        self.close();
        self.ui.tasks_open = true;
        self.ui.tasks_leaving = false;
    }
}

/// Installing takes two steps. Pick the game version in a full-page list.
/// The next page picks the loader, extra content and the install folder.
pub fn new_instance(g: &Gui) -> El<Gui> {
    let body = row().grow(1.0).min_height(0.0).fill_width().align(Align::Stretch).padding(Insets::new(12.0, 18.0, 16.0, 14.0)).gap(18.0);
    match g.ui.install.version.clone() {
        None => {
            let lead = row()
                .gap(4.0)
                .child(icon_button(Icon::ChevronLeft).label(tr!("common.back")).on_click(|g: &mut Gui| g.open(Sheet::Instances)))
                .child(label(tr!("instances.install_new_version")).title());
            frame(header(lead, Vec::new()), page_switch("install_versions", g.ui.install.next_step.is_some(), |g: &mut Gui| g.arrive_install_step(), body.child(versions(g))), None)
        }
        Some(mc) => {
            let lead = row()
                .gap(4.0)
                .child(icon_button(Icon::ChevronLeft).label(tr!("instances.reselect_version")).on_click(|g: &mut Gui| g.ui.install.next_step = Some(None)))
                .child(label(tr!("instances.install_mc", version = mc)).title());
            let loaders = scroll()
                .grow(1.0)
                .min_width(0.0)
                .align(Align::Stretch)
                .gap(8.0)
                .padding(Insets::new(0.0, 6.0, 0.0, 0.0))
                .child(label(tr!("instances.mod_loader")).secondary())
                .child(picker(g, PickTarget::Install, &mc, true));
            frame(
                header(lead, Vec::new()),
                page_switch(("install_loaders", mc), g.ui.install.next_step.is_some(), |g: &mut Gui| g.arrive_install_step(), body.child(loaders).child(options(g))),
                None,
            )
        }
    }
}

fn versions(g: &Gui) -> El<Gui> {
    let f = &g.ui.install;
    let filters = [
        (VersionFilter::Release, tr!("instances.filter_release")),
        (VersionFilter::Snapshot, tr!("instances.filter_snapshot")),
        (VersionFilter::Old, tr!("instances.filter_old")),
    ];
    let bar = row()
        .fill_width()
        .gap(2.0)
        .children(filters.map(|(v, name)| tab(name).selected(f.filter.target() == v).on_click(move |g: &mut Gui| g.ui.install.filter.go(v))))
        .child(row().grow(1.0))
        .child(
            text_input(&f.query)
                .icon(Icon::Search)
                .placeholder(tr!("instances.filter_placeholder"))
                .width(170.0)
                .on_change(|g: &mut Gui, s: String| g.ui.install.query = s),
        );
    let list: El<Gui> = match &g.m.manifest {
        Remote::Ready(all) => {
            let q = f.query.trim();
            let shown: Vec<&ManifestEntry> = all
                .iter()
                .filter(|e| match f.filter.shown {
                    VersionFilter::Release => e.kind == VersionType::Release,
                    VersionFilter::Snapshot => e.kind == VersionType::Snapshot,
                    VersionFilter::Old => matches!(e.kind, VersionType::OldBeta | VersionType::OldAlpha),
                })
                .filter(|e| q.is_empty() || e.id.contains(q))
                .collect();
            scroll()
                .grow(1.0)
                .align(Align::Stretch)
                .gap(2.0)
                .padding(Insets::new(0.0, 6.0, 0.0, 0.0))
                .children(shown.into_iter().map(|e| version_row(g, e)))
                .into()
        }
        Remote::Failed(e) => column()
            .grow(1.0)
            .child(empty_state(Icon::Warning, &tr!("instances.fetch_manifest_failed", error = e), ""))
            .child(button(tr!("common.retry")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.load_manifest(&mut g.m, true)))
            .into(),
        _ => column().grow(1.0).child(loading_row(tr!("instances.fetching_manifest"))).into(),
    };
    column().grow(1.0).min_width(0.0).gap(10.0).align(Align::Stretch).child(bar).child(page_switch(("version_filter", f.filter.shown as u8), f.filter.leaving(), |g: &mut Gui| {
        g.ui.install.filter.arrive();
    }, list)).into()
}

fn version_row(g: &Gui, e: &ManifestEntry) -> El<Gui> {
    let id = e.id.clone();
    let mut r = list_item()
        .key(e.id.clone())
        .padding(Insets::symmetric(12.0, 7.0))
        .selected(g.ui.install.version.as_deref() == Some(e.id.as_str()))
        .on_click(move |g: &mut Gui| g.choose_version(id.clone()))
        .child(label(&e.id).weight(700).color(fg(0.92)).grow(1.0));
    if g.m.latest_release.as_deref() == Some(e.id.as_str()) {
        r = r.child(tag(tr!("instances.latest")).accent());
    }
    r.child(label(format::iso_date(&e.release_time)).caption()).into()
}

fn options(g: &Gui) -> El<Gui> {
    let f = &g.ui.install;

    let folders = g.m.folders();
    let chosen = g.install_folder();
    let folder_label = |name: &str| folders.iter().find(|x| x.name == name).map(|x| x.label().to_string()).unwrap_or_else(|| name.to_string());
    let mut place = column().fill_width().gap(3.0).align(Align::Stretch).child(
        list_item()
            .padding(Insets::symmetric(10.0, 7.0))
            .gap(10.0)
            .selected(f.folder_open)
            .on_click(|g: &mut Gui| g.ui.install.folder_open = !g.ui.install.folder_open)
            .child(icon(folders.iter().find(|x| x.name == chosen).map(|x| folder_icon(x.kind)).unwrap_or(Icon::Folder)).icon_size(18.0))
            .child(label(folder_label(&chosen)).weight(700).color(fg(0.92)).grow(1.0).truncate())
            .child(icon(if f.folder_open { Icon::ChevronUp } else { Icon::ChevronDown }).icon_size(14.0)),
    );
    if f.folder_open {
        place = place.children(folders.iter().map(|x| {
            let name = x.name.clone();
            list_item()
                .key(("place", x.name.clone()))
                .padding(Insets::symmetric(10.0, 5.0))
                .gap(10.0)
                .selected(x.name == chosen)
                .on_click(move |g: &mut Gui| {
                    g.ui.install.folder = Some(name.clone());
                    g.ui.install.folder_open = false;
                })
                .child(icon(folder_icon(x.kind)).icon_size(16.0))
                .child(label(x.label()).color(fg(0.9)).grow(1.0).truncate())
        }));
    }

    let name_box = column().fill_width().gap(4.0).align(Align::Stretch).child(
        text_input(&f.name).placeholder(tr!("loaders.select_game_version_first")).on_change(|g: &mut Gui, s: String| {
            g.ui.install.name = s;
            g.ui.install.name_edited = true;
        }),
    );
    let name_box = name_box.child(label(g.name_problem().unwrap_or_default()).caption().min_height(16.0).color(RED));

    let form = scroll()
        .grow(1.0)
        .align(Align::Stretch)
        .gap(8.0)
        .child(label(tr!("instances.install_location")).secondary())
        .child(place)
        .child(label(tr!("instances.instance_name")).secondary().margin(Insets::new(6.0, 0.0, 0.0, 0.0)))
        .child(name_box);

    // The summary names the version that Auto will really install. If installing is not possible,
    // the reason is shown instead.
    let problem = g.install_problem();
    // A name problem is already shown under the name box, so it is not repeated here.
    let shown_problem = problem.clone().filter(|p| g.name_problem().as_ref() != Some(p));
    let summary = match (&shown_problem, &f.version) {
        (Some(p), _) => p.clone(),
        (None, Some(v)) => {
            let plan = loaders::plan_for(g, PickTarget::Install, v);
            let mut parts = vec![v.clone()];
            if let Some((k, ver)) = &plan.loader {
                parts.push(format!("{} {ver}", loader_name(*k)));
                if f.pick.api_mod && matches!(k, LoaderKind::Fabric | LoaderKind::Quilt) {
                    parts.push(if *k == LoaderKind::Fabric { "Fabric API" } else { "Quilt Standard Libraries" }.into());
                }
            }
            if let Some(of) = &plan.optifine {
                parts.push(format!("OptiFine {of}"));
            }
            format!("{} {}", tr!("common.install"), parts.join(" · "))
        }
        (None, None) => String::new(),
    };
    let summary_color = if shown_problem.is_some() && f.version.is_some() { RED } else { fg(0.6) };
    column()
        .width(300.0)
        .shrink(0.0)
        .gap(8.0)
        .align(Align::Stretch)
        .child(form)
        // Reserve two lines for the summary and the reason, which vary in length.
        // This keeps the install button from being pushed down.
        // The right edge lines up with the controls in the scroll area above.
        .child(
            column()
                .align(Align::Stretch)
                .gap(8.0)
                .padding(Insets::new(0.0, SCROLL_GUTTER, 0.0, 0.0))
                .child(label(summary).caption().color(summary_color).min_height(36.0))
                .child(
                    button(tr!("instances.start_install")).accent().icon(Icon::Download).enabled(problem.is_none()).on_click(|g: &mut Gui| g.start_install()),
                ),
        )
        .into()
}
