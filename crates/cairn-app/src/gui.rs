//! Application state and root view.
//! `Gui` pairs the model `m`, which only the bridge changes, with the UI state `ui`.
//! The root view stacks the title bar, the sheet area and the dock, with task popups on top.

use cairn_ui::Image;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use cairn_bridge::{Appearance, Bridge, ContentKind, CrashInfo, CrashKind, FOLDER_CURRENT, InstanceSettings, LoaderKind, Model, Remote, Status, ToastAction};
use cairn_ui::{Align, FileDrop, Insets, Justify, Point, Rect, Styled, Theme, View, click_area, column, reveal, row};

use crate::{board, style, views};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Sheet {
    Library,
    Instances,
    NewInstance,
    /// Settings of one instance: overview, launch options, loader and export.
    Version,
    Accounts,
    Settings,
    /// Modpack import. Shown based on the state of `m.pack`, not opened by a menu action.
    ImportPack,
    InitialSetup,
    /// Batch import of existing versions.
    ImportVersions,
    /// Standalone user rules and EULA panel.
    Eula,
    /// A panel defined by a user YAML file.
    CustomPanel(String),
    /// A panel a plugin provides, by plugin id and panel id. Opened by a button, never from the dock.
    PluginPanel(String, String),
    /// Crash panel, keyed by `CrashInfo::id`. Opens automatically when a new crash happens.
    Crash(u64),
    /// Software update prompt panel.
    Update,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsTab {
    #[default]
    Appearance,
    Game,
    Java,
    Download,
    /// Plugins, YAML widgets and YAML panels: the manager in [`crate::views::extensions`].
    Extensions,
    About,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AddAccountTab {
    /// Microsoft is listed first, so the add-account menu opens on it.
    #[default]
    Microsoft,
    Offline,
    Yggdrasil,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VersionPage {
    #[default]
    Overview,
    /// Java, memory, arguments and so on. Each item can follow the global settings.
    Launch,
    Loader,
    Export,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VersionFilter {
    #[default]
    Release,
    Snapshot,
    Old,
}

/// A removal that runs after the row's collapse animation ends.
/// The row collapses first (rows below slide up), then it is really deleted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Removal {
    Content(PathBuf),
    Account(String),
    Instance(String),
    FinishedTasks,
    /// A crash card in the notification center, by `CrashInfo::id`.
    Crash(u64),
}

/// State for in-place page switches: settings tabs, instance subpages, content categories.
/// The old page plays its leave animation first. The tab highlight moves to the target at once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Paged<T> {
    pub shown: T,
    next: Option<T>,
}

impl<T: Copy + PartialEq> Paged<T> {
    pub fn new(page: T) -> Self {
        Paged { shown: page, next: None }
    }

    /// The page whose tab should be highlighted.
    pub fn target(&self) -> T {
        self.next.unwrap_or(self.shown)
    }

    pub fn leaving(&self) -> bool {
        self.next.is_some()
    }

    /// Goes to another page. Going back to the shown page cancels the leave in progress.
    pub fn go(&mut self, page: T) {
        self.next = (page != self.shown).then_some(page);
    }

    /// Called when the leave animation ends. Switches to the new page and returns it.
    pub fn arrive(&mut self) -> Option<T> {
        let next = self.next.take()?;
        self.shown = next;
        Some(next)
    }
}

/// A component version. `Auto` is resolved to a concrete version before submitting.
/// The version is picked by [`crate::views::loader_plan`] using compatibility rules.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum VersionChoice {
    #[default]
    Auto,
    Exact(String),
}

impl VersionChoice {
    pub fn exact(&self) -> Option<&str> {
        match self {
            VersionChoice::Auto => None,
            VersionChoice::Exact(v) => Some(v),
        }
    }
}

/// State of the loader picker, shared by the install page and the modify page.
#[derive(Clone, Debug, PartialEq)]
pub struct LoaderPick {
    /// Main loader: at most one of Fabric, Quilt, Forge or NeoForge. `None` is vanilla.
    pub loader: Option<LoaderKind>,
    pub loader_version: VersionChoice,
    /// OptiFine to install alongside (install page only). `None` skips it.
    pub optifine: Option<VersionChoice>,
    /// With Fabric or Quilt, also install the matching API mod (Fabric API or QSL).
    pub api_mod: bool,
    /// The row whose version list is expanded. Only affects the display.
    pub open: Option<LoaderKind>,
}

impl Default for LoaderPick {
    fn default() -> Self {
        Self { loader: None, loader_version: VersionChoice::Auto, optifine: None, api_mod: true, open: None }
    }
}

impl LoaderPick {
    pub fn of_instance(inst: &cairn_bridge::Instance) -> LoaderPick {
        let current = inst.loaders.iter().find(|l| l.kind != LoaderKind::OptiFine && l.kind != LoaderKind::LiteLoader);
        LoaderPick {
            loader: current.map(|l| l.kind),
            loader_version: current.map_or(VersionChoice::Auto, |l| VersionChoice::Exact(l.version.clone())),
            ..LoaderPick::default()
        }
    }
}

#[derive(Debug, Default)]
pub struct InstallForm {
    pub filter: Paged<VersionFilter>,
    pub query: String,
    pub version: Option<String>,
    pub pick: LoaderPick,
    pub name: String,
    /// Once the user edits the name, changing the version or loader stops renaming it.
    pub name_edited: bool,
    /// Game folder to install into, by registered name. An empty string means the standalone
    /// instance library. `None` uses the default folder.
    pub folder: Option<String>,
    pub folder_open: bool,
    /// Step to move to next: `Some(Some(v))` picks version `v` and goes on to the loader step.
    /// `Some(None)` goes back to the version step. The current step fades out before the switch.
    pub next_step: Option<Option<String>>,
}

/// Text being edited on the instance overview page. It is written to disk only on Save.
#[derive(Debug, Default)]
pub struct Draft {
    pub name: String,
    pub note: String,
}

#[derive(Debug, Default)]
pub struct ExportForm {
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Checked paths. `None` means the user has not changed them, so the defaults apply.
    pub include: Option<HashSet<String>>,
    pub for_instance: Option<String>,
}

#[derive(Debug, Default)]
pub struct PackForm {
    /// Name the user typed. `None` uses the modpack's own name.
    pub name: Option<String>,
    /// Game folder to import into, by registered name. An empty string means the standalone
    /// instance library. `None` uses the default folder.
    pub folder: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    Content(PathBuf),
    Instance(String),
    Account(String),
    /// Removes the registration of a game folder.
    Folder(String),
    /// Force-quits a running game, by `RunningGame::id`.
    KillGame(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceMenu {
    pub id: String,
    /// Where the right button was released, in window coordinates.
    pub at: Point,
    /// Set when 'Delete instance...' is clicked. The menu then shows a confirmation.
    pub confirm: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DropdownKind {
    Language,
    /// The Add menu of the plugin manager: new plugin, install, open folders.
    AddExtension,
    /// Links for an online mod/item (platform page and MCMod wiki).
    OnlineLinks {
        project_id: String,
        source: cairn_bridge::OnlineSource,
        platform_url: String,
        wiki_url: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveDropdown {
    pub kind: DropdownKind,
    pub at: Point,
    pub align_right: bool,
    pub min_width: f32,
}

/// Inputs that decide the theme. The theme is rebuilt only when one of them changes.
#[derive(Clone, Debug, PartialEq)]
struct ThemeKey {
    appearance: Appearance,
    /// Path of the decoded wallpaper and the address of its pixel buffer, which changes
    /// when the image is decoded again.
    wallpaper: Option<(PathBuf, usize)>,
    system_light: bool,
}

#[derive(Debug, Default)]
pub struct Ui {
    pub sheet: Option<Sheet>,
    pub leaving: Option<Sheet>,
    /// Next sheet to open when switching. It enters after the leaving sheet finishes its animation.
    pub pending_sheet: Option<Sheet>,
    pub tasks_open: bool,
    pub tasks_leaving: bool,
    /// Finished tasks whose banner has already collapsed.
    pub banner_done: HashSet<u64>,
    /// Crashes cleared from the notification center, by `CrashInfo::id`. The card is no longer
    /// listed, but the record stays and the crash panel can still be opened.
    pub crash_hidden: HashSet<u64>,
    pub update_prompted_version: Option<String>,
    pub settings_tab: Paged<SettingsTab>,
    pub extensions: views::extensions::State,
    pub accent_picker: Option<views::accent_picker::State>,
    pub accent_picker_leaving: Option<views::accent_picker::State>,
    pub tab: Paged<ContentKind>,
    pub search: String,
    /// How many local content items are shown now. Items load lazily; the default is 40.
    pub content_page_limit: usize,
    pub install: InstallForm,
    pub offline_name: String,
    pub ygg_server: String,
    pub ygg_custom: bool,
    pub ygg_username: String,
    pub ygg_password: String,
    pub ygg_password_shown: bool,
    pub add_account_open: bool,
    pub add_account_leaving: bool,
    pub add_account_tab: AddAccountTab,
    pub setup_eula_agreed: bool,
    /// The first-run agreement text has been scrolled to its end. The consent controls stay locked until then.
    pub legal_read: bool,
    pub setup_finished: bool,
    pub import_selected: Option<HashSet<String>>,
    pub confirm: Option<Confirm>,
    pub instance_menu: Option<InstanceMenu>,
    pub instance_menu_leaving: Option<InstanceMenu>,
    pub dropdown: Option<ActiveDropdown>,
    pub dropdown_leaving: Option<ActiveDropdown>,
    /// The value picked in the dropdown. It is applied after the menu closes, since a language
    /// change re-lays out the whole UI.
    pub dropdown_choice: Option<cairn_bridge::Language>,
    pub removing: Vec<Removal>,
    pub drop_hover: bool,
    pub categories_open: bool,
    pub categories_leaving: bool,
    /// Game folder shown in the instance list, by registered name. An empty string is the
    /// standalone instance library. `None` follows the selected instance.
    pub folder: Option<String>,
    pub board: board::BoardUi,
    /// Instance that the instance settings panel is for.
    pub version_id: Option<String>,
    pub version_page: Paged<VersionPage>,
    pub draft: Draft,
    pub modify: LoaderPick,
    pub export: ExportForm,
    pub pack: PackForm,
    /// The newest crash that already popped up automatically, by `CrashInfo::id`.
    pub crash_shown: u64,
    pub crash_lines_open: bool,
}

pub struct Gui {
    pub m: Model,
    pub bridge: Bridge,
    pub ui: Ui,
    theme_key: Option<ThemeKey>,
    theme_scene: Option<Rc<dyn cairn_ui::Scene>>,
    icon_cache: RefCell<HashMap<String, (Arc<Vec<u8>>, Arc<Image>)>>,
}

/// Largest width and height of a sheet, in logical pixels. Smaller windows shrink it.
const SHEET_WIDTH: f32 = 800.0;
const SHEET_HEIGHT: f32 = 560.0;
/// The settings panel is larger than the other sheets: its Plugins tab holds a list and a detail pane side by side.
const SETTINGS_WIDTH: f32 = 880.0;
const SETTINGS_HEIGHT: f32 = 640.0;

impl Gui {
    pub fn new(bridge: Bridge) -> Self {
        Gui { m: Model::default(), bridge, ui: Ui::default(), theme_key: None, theme_scene: None, icon_cache: RefCell::new(HashMap::new()) }
    }

    pub fn copy_text(&mut self, text: &str) {
        if self.bridge.copy_to_clipboard(text) {
            self.bridge.toast(&mut self.m, cairn_bridge::ToastKind::Success, cairn_i18n::tr("common.copied"));
        } else {
            self.bridge.toast(&mut self.m, cairn_bridge::ToastKind::Error, cairn_i18n::tr("common.copy_failed"));
        }
    }

    /// Returns the image for `key`. The converted image is cached and reused while the pixel buffer stays the same.
    pub fn icon(&self, key: &str) -> Option<Arc<Image>> {
        let icon_img = self.m.icons.get(key)?;
        let mut cache = self.icon_cache.borrow_mut();
        if let Some((source, img)) = cache.get(key)
            && Arc::ptr_eq(source, &icon_img.rgba)
        {
            return Some(img.clone());
        }
        let image = Image::from_rgba8(icon_img.width, icon_img.height, (*icon_img.rgba).clone())?;
        let arc = Arc::new(image);
        cache.insert(key.to_string(), (icon_img.rgba.clone(), arc.clone()));
        Some(arc)
    }

    /// The sheet to show now. While a modpack import is pending, the import dialog covers the others.
    fn shown_sheet(&self) -> Option<Sheet> {
        if !self.m.config.setup_completed && !self.ui.setup_finished {
            return Some(Sheet::InitialSetup);
        }
        if !matches!(self.m.pack, Remote::Idle) { Some(Sheet::ImportPack) } else { self.ui.sheet.clone() }
    }

    pub fn view(&self) -> impl View<Self> + use<> {
        cairn_i18n::set_language(self.m.appearance.language);
        // Keep the home page hidden while switching sheets, because the next sheet is already queued.
        let sheet_active = self.shown_sheet().is_some() || self.ui.pending_sheet.is_some();
        let sheet = match (self.shown_sheet(), self.ui.leaving.clone()) {
            (Some(s), _) => Some(sheet_reveal(self, s)),
            (None, Some(s)) => Some(sheet_reveal(self, s).leaving(true).on_left(|g: &mut Gui| {
                g.ui.leaving = None;
                if let Some(next) = g.ui.pending_sheet.take() {
                    g.ui.sheet = Some(next);
                }
            })),
            _ => None,
        };

        let mut middle = column().grow(1.0).fill_width().min_height(0.0);

        let dashboard_el = reveal()
            .key("dashboard")
            .leaving(sheet_active)
            .duration(std::time::Duration::from_millis(220))
            .exit_duration(std::time::Duration::from_millis(220))
            .offset(16.0)
            .absolute()
            .top(0.0)
            .left(0.0)
            .right(0.0)
            .bottom(0.0)
            .child(column().fill_width().fill_height().align(Align::Center).padding(Insets::new(0.0, 28.0, 10.0, 28.0)).child(board::view(self)));
        middle = middle.child(dashboard_el);

        if let Some(s) = sheet {
            let sheet_container =
                column().fill_width().fill_height().align(Align::Center).justify(Justify::End).padding(Insets::new(10.0, 36.0, 16.0, 36.0)).child(s);
            middle = middle.child(sheet_container);
        }
        // While the home page is edited, the dock fades out but keeps its space, so the grid cells do not grow.
        let dock = reveal().key("dock").leaving(self.ui.board.editing).offset(24.0).child(views::dock(self));
        let mut root = column()
            .fill_width()
            .fill_height()
            .child(views::title_bar(self).shrink(0.0))
            .child(middle)
            .child(row().fill_width().shrink(0.0).justify(Justify::Center).padding(Insets::new(0.0, 24.0, 22.0, 24.0)).child(dock));
        root = root.children(board::overlays(self));
        root = root.children(views::notification_banners(self));
        // A transparent full-window layer under the popup closes it when clicked outside.
        if self.ui.tasks_open {
            root = root.child(click_area().absolute().top(0.0).right(0.0).bottom(0.0).left(0.0).on_click(|g: &mut Gui| g.close_tasks()));
        }
        if self.ui.tasks_open || self.ui.tasks_leaving {
            root = root.child(
                reveal()
                    .from_above()
                    .offset(8.0)
                    .duration(std::time::Duration::from_millis(160))
                    .exit_duration(std::time::Duration::from_millis(120))
                    .absolute()
                    .top(54.0)
                    .right(14.0)
                    .width(views::NOTIFICATION_WIDTH)
                    .key("tasks")
                    .leaving(!self.ui.tasks_open)
                    .on_left(|g: &mut Gui| g.ui.tasks_leaving = false)
                    .child(views::tasks_popover(self)),
            );
        }
        if let Some(menu) = &self.ui.instance_menu {
            root = root.child(views::instance_menu(self, menu, false));
        } else if let Some(menu) = &self.ui.instance_menu_leaving {
            root = root.child(views::instance_menu(self, menu, true));
        }
        let drop_target = self.ui.dropdown.as_ref().or(self.ui.dropdown_leaving.as_ref());
        if let Some(drop) = drop_target {
            let leaving = self.ui.dropdown.is_none();
            root = root.child(views::dropdown_view(self, drop, leaving));
        }
        if let Some(picker) = self.ui.accent_picker.as_ref().or(self.ui.accent_picker_leaving.as_ref()) {
            root = root.child(views::accent_picker::view(self, picker, self.ui.accent_picker.is_none()));
        }
        root
    }

    /// The open sheet. While switching, this is the queued sheet, which the dock highlights.
    pub fn active_sheet(&self) -> Option<&Sheet> {
        self.ui.sheet.as_ref().or(self.ui.pending_sheet.as_ref())
    }

    pub fn toggle(&mut self, sheet: Sheet) {
        if self.active_sheet() == Some(&sheet) {
            self.close();
        } else {
            self.open(sheet);
        }
    }

    pub fn open(&mut self, sheet: Sheet) {
        views::accent_picker::close(self);
        self.save_draft();
        self.flush_removals();
        self.ui.board.exit_edit();
        self.close_categories();
        self.close_add_account();
        self.ui.confirm = None;
        self.close_instance_menu();
        self.close_dropdown();
        // Another sheet is open or closing. Let it finish its leave animation before the new one.
        if self.ui.sheet.as_ref().is_some_and(|s| *s != sheet) {
            self.ui.leaving = self.ui.sheet.take();
            self.ui.pending_sheet = Some(sheet.clone());
        } else if self.ui.sheet.is_none() && self.ui.leaving.is_some() {
            self.ui.pending_sheet = Some(sheet.clone());
        } else {
            self.ui.sheet = Some(sheet.clone());
        }
        if self.m.status != Status::Ready {
            return;
        }
        match sheet {
            Sheet::Library => {
                if let Some(id) = self.m.selected.clone() {
                    self.ui.search.clear();
                    self.bridge.open_content(&mut self.m, &id, self.ui.tab.shown);
                }
            }
            Sheet::NewInstance => {
                if !matches!(self.m.manifest, Remote::Ready(_) | Remote::Loading) {
                    self.bridge.load_manifest(&mut self.m, false);
                }
            }
            Sheet::Settings => {
                if matches!(self.m.javas, Remote::Idle) {
                    self.bridge.load_javas(&mut self.m, false);
                }
                self.bridge.refresh_memory(&mut self.m, None);
            }
            Sheet::Instances => {
                self.ui.folder = None;
                self.bridge.refresh_instances(&mut self.m);
            }
            Sheet::ImportVersions => self.bridge.scan_import_candidates(&mut self.m, None),
            Sheet::Version
            | Sheet::Accounts
            | Sheet::ImportPack
            | Sheet::InitialSetup
            | Sheet::Eula
            | Sheet::CustomPanel(_)
            | Sheet::PluginPanel(..)
            | Sheet::Crash(_)
            | Sheet::Update => {}
        }
    }

    pub fn close(&mut self) {
        views::accent_picker::close(self);
        self.save_draft();
        self.flush_removals();
        self.close_add_account();
        self.ui.pending_sheet = None;
        if self.ui.sheet.is_some() {
            self.ui.leaving = self.ui.sheet.take();
        }
        self.ui.confirm = None;
        self.close_dropdown();
    }

    pub fn open_dropdown(&mut self, kind: DropdownKind, rect: Rect) {
        if self.ui.dropdown.as_ref().map(|d| &d.kind) == Some(&kind) {
            self.close_dropdown();
        } else {
            let align_right = rect.x > 260.0;
            let at_x = if align_right { rect.x + rect.width } else { rect.x };
            // The previous menu is still closing, and its close callback will not run.
            // So the picked value is applied now.
            self.ui.dropdown_leaving = None;
            self.apply_dropdown_choice();
            self.ui.dropdown = Some(ActiveDropdown { kind, at: Point::new(at_x, rect.y + rect.height + 4.0), align_right, min_width: rect.width.max(180.0) });
        }
    }

    /// Collapses the row first and removes it afterwards (see [`Removal`]). Only use it for a row
    /// that is on screen now.
    pub fn remove_after_collapse(&mut self, r: Removal) {
        if !self.ui.removing.contains(&r) {
            self.ui.removing.push(r);
        }
    }

    pub fn is_removing(&self, r: &Removal) -> bool {
        self.ui.removing.contains(r)
    }

    /// Crashes listed in the notification center: game crashes not yet cleared, newest first.
    /// Launch failures are not listed here. They show on their task's card.
    pub fn listed_crashes(&self) -> impl Iterator<Item = &CrashInfo> {
        self.m.crashes.iter().rev().filter(|c| matches!(c.kind, CrashKind::Game { .. }) && !self.ui.crash_hidden.contains(&c.id))
    }

    /// Collapses finished tasks and crash cards, then clears them once the collapse ends.
    /// Only rows listed now are collapsed. A row that is not on screen never finishes its
    /// collapse, so it would be cleared as soon as it appears.
    pub fn clear_finished(&mut self) {
        let crashes: Vec<u64> = self.listed_crashes().map(|c| c.id).collect();
        for id in crashes {
            self.remove_after_collapse(Removal::Crash(id));
        }
        if self.m.tasks.iter().any(|t| !t.is_running()) {
            self.remove_after_collapse(Removal::FinishedTasks);
        }
    }

    pub fn finish_removal(&mut self, r: &Removal) {
        let Some(i) = self.ui.removing.iter().position(|x| x == r) else { return };
        let r = self.ui.removing.remove(i);
        match r {
            Removal::Content(p) => self.bridge.delete_content(&mut self.m, &p),
            Removal::Account(id) => self.bridge.remove_account(&mut self.m, &id),
            Removal::Instance(id) => self.bridge.delete_instance(&mut self.m, &id),
            Removal::FinishedTasks => self.bridge.clear_finished_tasks(&mut self.m),
            Removal::Crash(id) => {
                self.ui.crash_hidden.insert(id);
                // The crash banner is the same notification, so clear it too. Otherwise the cleared one
                // pops back in the top right corner when the center closes.
                let banners: Vec<u64> = self.m.toasts.iter().filter(|t| t.action == Some(ToastAction::ShowCrash(id))).map(|t| t.id).collect();
                for toast in banners {
                    self.bridge.dismiss_toast(&mut self.m, toast);
                }
            }
        }
    }

    /// The panel was closed, so collapse animations will not finish. Run pending removals now.
    fn flush_removals(&mut self) {
        for r in std::mem::take(&mut self.ui.removing) {
            self.ui.removing.push(r.clone());
            self.finish_removal(&r);
        }
    }

    pub fn apply_dropdown_choice(&mut self) {
        if let Some(l) = self.ui.dropdown_choice.take() {
            let mut ap = self.m.appearance.clone();
            ap.language = l;
            self.bridge.set_appearance(&mut self.m, ap);
        }
    }

    pub fn close_dropdown(&mut self) {
        if let Some(drop) = self.ui.dropdown.take() {
            self.ui.dropdown_leaving = Some(drop);
        }
    }

    pub fn open_add_account(&mut self) {
        if matches!(self.m.ms_login, cairn_bridge::MsLogin::Succeeded { .. }) {
            self.bridge.cancel_ms_login(&mut self.m);
        }
        // Always start on the first tab (Microsoft), not on the tab used last time.
        self.ui.add_account_tab = AddAccountTab::default();
        self.ui.add_account_open = true;
        self.ui.add_account_leaving = false;
    }

    /// Closes the add-account panel. A typed password is dropped, so the next open starts empty.
    pub fn close_add_account(&mut self) {
        if self.ui.add_account_open {
            self.ui.add_account_open = false;
            self.ui.add_account_leaving = true;
        }
        self.ui.ygg_password.clear();
        self.ui.ygg_password_shown = false;
    }

    pub fn toggle_add_account(&mut self) {
        if self.ui.add_account_open {
            self.close_add_account();
        } else {
            self.open_add_account();
        }
    }

    /// The title bar close button. Closes the import dialog while it is shown, else the current sheet.
    pub fn close_current(&mut self) {
        // Until the first-run setup is done, its panel is forced on screen.
        // Closing it would skip the import, and it cannot be closed before the EULA is accepted.
        if self.shown_sheet() == Some(Sheet::InitialSetup) {
            if self.m.config.eula_accepted {
                self.skip_setup_import();
            }
            return;
        }
        if matches!(self.m.pack, Remote::Idle) {
            self.close();
        } else {
            self.close_pack();
        }
    }

    pub fn close_pack(&mut self) {
        self.bridge.dismiss_pack(&mut self.m);
        self.ui.pack = PackForm::default();
        self.ui.leaving = Some(Sheet::ImportPack);
    }

    pub fn toggle_tasks(&mut self) {
        if self.ui.tasks_open {
            self.close_tasks();
        } else {
            self.ui.tasks_open = true;
            self.ui.tasks_leaving = false;
        }
    }

    pub fn close_tasks(&mut self) {
        if self.ui.tasks_open {
            self.ui.tasks_open = false;
            self.ui.tasks_leaving = true;
        }
    }

    pub fn toggle_categories(&mut self) {
        if self.ui.categories_open {
            self.close_categories();
        } else {
            self.ui.categories_open = true;
            self.ui.categories_leaving = false;
        }
    }

    pub fn close_categories(&mut self) {
        if self.ui.categories_open {
            self.ui.categories_open = false;
            self.ui.categories_leaving = true;
        }
    }

    pub fn current_folder(&self) -> String {
        if let Some(f) = &self.ui.folder {
            return f.clone();
        }
        self.m.selected_instance().map(|i| self.m.folder_of(i).to_string()).unwrap_or_else(|| FOLDER_CURRENT.to_string())
    }

    pub fn open_instance_menu(&mut self, id: &str, at: Point) {
        self.ui.instance_menu_leaving = None;
        self.ui.instance_menu = Some(InstanceMenu { id: id.to_string(), at, confirm: false });
    }

    /// Closes the instance menu. It is removed once its fade-out ends.
    pub fn close_instance_menu(&mut self) {
        if let Some(m) = self.ui.instance_menu.take() {
            self.ui.instance_menu_leaving = Some(m);
        }
    }

    /// Called after the bridge changes the model. A new crash or launch failure opens the crash panel.
    pub fn after_update(&mut self) {
        if let Some((page, plugin)) = self.m.plugins.open_page.take() {
            // A plugin that was just created or installed appears once the rescan finishes.
            if page == "plugin" && self.m.plugins.busy && self.m.plugins.entry(&plugin).is_none() {
                self.m.plugins.open_page = Some((page, plugin));
            } else {
                self.open_plugin_page(&page, &plugin);
            }
        }
        self.ui.banner_done.retain(|id| self.m.tasks.iter().any(|task| task.id == *id));
        self.ui.crash_hidden.retain(|id| self.m.crashes.iter().any(|c| c.id == *id));
        if !self.m.icons.is_empty() || !self.icon_cache.get_mut().is_empty() {
            let mut active = HashSet::new();
            for account in self.m.accounts.ready().into_iter().flatten() {
                active.insert(format!("avatar:{}", account.id));
                active.insert(format!("avatar:{}", account.name));
            }
            for item in self.m.content.items.ready().into_iter().flatten() {
                active.insert(item.path.to_string_lossy().into_owned());
            }
            for item in self.m.online.results.ready().into_iter().flatten() {
                if let Some(url) = &item.icon_url {
                    active.insert(url.clone());
                }
            }
            // Pictures in YAML and plugin views are keyed by their URL too. Without this they would be dropped the moment they load.
            views::custom_panel::view_images(self, &mut active);
            self.m.icons.retain(|key, _| active.contains(key));
            self.icon_cache.get_mut().retain(|key, (source, _)| self.m.icons.get(key).is_some_and(|icon| Arc::ptr_eq(source, &icon.rgba)));
        }
        let Some(latest) = self.m.crashes.last().map(|c| c.id) else {
            if let cairn_bridge::UpdateStatus::Available(info) = &self.m.update_status {
                if self.ui.update_prompted_version.as_deref() != Some(&info.target_version) {
                    self.ui.update_prompted_version = Some(info.target_version.clone());
                    self.open(Sheet::Update);
                }
            }
            return;
        };
        if latest > self.ui.crash_shown {
            self.open_crash(latest);
        }
        if let cairn_bridge::UpdateStatus::Available(info) = &self.m.update_status {
            if self.ui.update_prompted_version.as_deref() != Some(&info.target_version) {
                self.ui.update_prompted_version = Some(info.target_version.clone());
                self.open(Sheet::Update);
            }
        }
    }

    pub fn open_crash(&mut self, id: u64) {
        self.ui.crash_shown = self.ui.crash_shown.max(id);
        // A launch failure has only a line or two of error, so show it expanded at once.
        self.ui.crash_lines_open = self.m.crash(id).is_some_and(|c| c.is_launch_failure());
        self.close_tasks();
        self.open(Sheet::Crash(id));
    }

    /// Selects and launches the instance. With no account yet, the accounts panel opens first.
    pub fn launch_instance(&mut self, id: &str) {
        self.bridge.select_instance(&mut self.m, id);
        if self.m.selected_account().is_none() {
            self.open(Sheet::Accounts);
        } else {
            self.bridge.launch(&mut self.m, id);
            self.close();
        }
    }

    /// Saves the name or note the user edited but did not save when leaving the panel.
    /// An empty name is not saved.
    fn save_draft(&mut self) {
        if self.ui.sheet != Some(Sheet::Version) {
            return;
        }
        let Some(id) = self.ui.version_id.clone() else { return };
        let Some(inst) = self.m.instance(&id) else { return };
        let (name, note) = (self.ui.draft.name.trim().to_string(), self.ui.draft.note.trim().to_string());
        let shown_name = inst.settings.display_name.clone().unwrap_or_else(|| inst.name.clone());
        let shown_note = inst.settings.note.clone().unwrap_or_default();
        if name.is_empty() || (name == shown_name && note == shown_note) {
            return;
        }
        self.edit_instance(&id, move |s| {
            s.display_name = Some(name);
            s.note = (!note.is_empty()).then_some(note);
        });
    }

    pub fn open_version(&mut self, id: &str) {
        self.save_draft();
        let Some(inst) = self.m.instance(id) else { return };
        self.ui.draft =
            Draft { name: inst.settings.display_name.clone().unwrap_or_else(|| inst.name.clone()), note: inst.settings.note.clone().unwrap_or_default() };
        self.ui.modify = LoaderPick::of_instance(inst);
        self.ui.export = ExportForm::default();
        self.ui.version_id = Some(id.to_string());
        self.ui.version_page = Paged::new(VersionPage::Overview);
        self.open(Sheet::Version);
    }

    /// Switches the instance settings panel to another page and loads the data that page needs.
    pub fn set_version_page(&mut self, page: VersionPage) {
        self.ui.version_page.go(page);
        let Some(id) = self.ui.version_id.clone() else { return };
        match page {
            VersionPage::Overview => {}
            VersionPage::Launch => {
                if matches!(self.m.javas, Remote::Idle) {
                    self.bridge.load_javas(&mut self.m, false);
                }
                self.bridge.refresh_memory(&mut self.m, Some(&id));
            }
            VersionPage::Loader => {
                // The manifest decides which loaders support this game version. Load it if it is still idle.
                if matches!(self.m.manifest, Remote::Idle) {
                    self.bridge.load_manifest(&mut self.m, false);
                }
                if let Some(mc) = self.m.instance(&id).and_then(|i| i.mc_version.clone()) {
                    self.bridge.load_loaders(&mut self.m, &mc);
                }
            }
            VersionPage::Export => {
                self.bridge.scan_export(&mut self.m, &id);
                if self.ui.export.for_instance.as_deref() != Some(id.as_str()) {
                    let name = self.m.instance(&id).map(|i| i.name.clone()).unwrap_or_default();
                    self.ui.export = ExportForm { name, version: "1.0.0".into(), for_instance: Some(id), ..ExportForm::default() };
                }
            }
        }
    }

    /// Edits an instance's settings. `None` means the global value is used. The change is made on a
    /// copy and handed to the bridge to save.
    pub fn edit_instance(&mut self, id: &str, f: impl FnOnce(&mut InstanceSettings)) {
        let Some(inst) = self.m.instance(id) else { return };
        let mut settings = inst.settings.clone();
        f(&mut settings);
        self.bridge.save_instance_settings(&mut self.m, id, settings);
    }

    /// Opens a launcher page that a plugin asked for with `cairn.ui.open`.
    fn open_plugin_page(&mut self, page: &str, plugin: &str) {
        match page {
            "settings" => self.open(Sheet::Settings),
            "extensions" => crate::views::extensions::open(self, None),
            "plugin" => crate::views::extensions::open_plugin(self, plugin),
            // `cairn.panels.open(id)` asks for `panel:<id>`.
            _ if page.starts_with("panel:") => {
                let panel = &page["panel:".len()..];
                if self.m.plugins.panel(plugin, panel).is_some() {
                    self.open(Sheet::PluginPanel(plugin.to_string(), panel.to_string()));
                }
            }
            "instances" => self.open(Sheet::Instances),
            "content" => self.open(Sheet::Library),
            "accounts" => self.open(Sheet::Accounts),
            _ => {}
        }
    }

    pub fn on_file_drop(&mut self, drop: FileDrop) {
        match drop {
            FileDrop::Hovered(_) => self.ui.drop_hover = true,
            FileDrop::Cancelled => self.ui.drop_hover = false,
            FileDrop::Dropped(path) => {
                self.ui.drop_hover = false;
                let image = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"));
                let in_settings = self.ui.sheet == Some(Sheet::Settings);
                // Dropping an image on the appearance settings sets it as the wallpaper. Other files go to the
                // bridge, which recognizes modpacks, mods and resource packs.
                // Dropping a folder or .zip on the plugins page installs it as a plugin.
                if in_settings && self.ui.settings_tab.shown == SettingsTab::Extensions {
                    self.bridge.install_plugin(&mut self.m, path);
                } else if image && in_settings && self.ui.settings_tab.shown == SettingsTab::Appearance {
                    let mut ap = self.m.appearance.clone();
                    ap.wallpaper = Some(path);
                    self.bridge.set_appearance(&mut self.m, ap);
                } else {
                    self.bridge.import_files(&mut self.m, vec![path]);
                }
            }
        }
    }

    /// Builds a new theme when the appearance state changed. `App::theme_from` calls this after
    /// every state change.
    pub fn theme_if_changed(&mut self) -> Option<Theme> {
        let wallpaper = match (&self.m.appearance.wallpaper, &self.m.wallpaper) {
            (Some(_), Remote::Ready(w)) => Some(w),
            _ => None,
        };
        let mut appearance = self.m.appearance.clone();
        if let Some(picker) = &self.ui.accent_picker {
            let c = picker.preview_color();
            appearance.accent = cairn_bridge::AccentChoice::Custom([c.r, c.g, c.b]);
        }
        let key = ThemeKey {
            appearance: appearance.clone(),
            wallpaper: wallpaper.map(|w| (w.path.clone(), std::sync::Arc::as_ptr(&w.rgba) as usize)),
            system_light: self.m.system_light,
        };
        if self.theme_key.as_ref() == Some(&key) {
            return None;
        }
        let same_scene = self.theme_key.as_ref().is_some_and(|previous| {
            let mut next = key.clone();
            next.appearance.accent = previous.appearance.accent;
            previous == &next
        });
        let mut theme = style::theme(&appearance, wallpaper, self.m.system_light);
        // A color preview reuses the background scene, so the scene and the blur cache stay valid.
        if same_scene {
            theme.scene = self.theme_scene.clone();
        } else {
            self.theme_scene = theme.scene.clone();
        }
        self.theme_key = Some(key);
        Some(theme)
    }
}

/// Wraps a sheet in its show and hide transition.
/// The import dialog is small and does not fill the height.
fn sheet_reveal(g: &Gui, s: Sheet) -> cairn_ui::Reveal<Gui> {
    let r = reveal().fill_width().key(("sheet", s.clone()));
    // The sheet height depends only on the window: it fills the middle, up to SHEET_HEIGHT.
    // If it grew with the content, long pages would pass the limit and the panel would jump.
    // The import dialog is small, so its height follows the content.
    let r = match &s {
        Sheet::ImportPack => r.max_width(540.0),
        Sheet::Crash(_) => r.max_width(680.0).fill_height().max_height(SHEET_HEIGHT),
        Sheet::Update => r.max_width(560.0).fill_height().max_height(SHEET_HEIGHT),
        Sheet::Settings => r.max_width(SETTINGS_WIDTH).fill_height().max_height(SETTINGS_HEIGHT),
        _ => r.max_width(SHEET_WIDTH).fill_height().max_height(SHEET_HEIGHT),
    };
    r.child(views::sheet(g, s))
}

#[cfg(test)]
mod bench_anim {
    use super::*;

    fn ready_gui(rt: &tokio::runtime::Runtime) -> Gui {
        // Background callbacks are dropped: only the test changes the model.
        let sink: cairn_bridge::Sink = Arc::new(|_| true);
        let mut gui = Gui::new(Bridge::start(rt.handle().clone(), sink));
        gui.m.status = Status::Ready;
        gui.m.config.eula_accepted = true;
        // The instance list is already loaded (empty), so the launch button does not wait in a shimmer.
        gui.m.instances = Remote::Ready(Vec::new());
        gui
    }

    #[test]
    fn icon_caches_follow_current_accounts_and_replaced_pixels() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = ready_gui(&rt);
        let account = cairn_bridge::AccountInfo {
            id: "a".into(),
            name: "Alex".into(),
            kind: cairn_bridge::AccountKind::Offline,
            uuid: String::new(),
            skin_url: None,
            selected: true,
            needs_refresh: false,
            yggdrasil_server: None,
        };
        g.m.accounts = Remote::Ready(vec![account]);
        let pixels = || cairn_bridge::IconImage { width: 1, height: 1, rgba: Arc::new(vec![255; 4]) };
        for key in ["avatar:a", "avatar:Alex", "stale-search-result"] {
            g.m.icons.insert(key.into(), pixels());
            assert!(g.icon(key).is_some());
        }
        g.after_update();
        assert_eq!(g.m.icons.len(), 2);
        assert_eq!(g.icon_cache.borrow().len(), 2);
        g.m.icons.insert("avatar:a".into(), pixels());
        g.after_update();
        assert!(!g.icon_cache.borrow().contains_key("avatar:a"));
        assert!(g.icon("avatar:a").is_some());
        g.m.accounts = Remote::Ready(Vec::new());
        g.after_update();
        assert!(g.m.icons.is_empty());
        assert!(g.icon_cache.borrow().is_empty());
    }

    #[test]
    fn pictures_in_plugin_views_stay_while_a_view_names_them() {
        use cairn_bridge::{CustomPanelDef, LiveWidget, PanelItem, PluginPanel};
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = ready_gui(&rt);
        let image = |url: &str| PanelItem::Image { url: url.into(), height: None, radius: None };
        let key = cairn_bridge::panel_key("news", "main");
        let declared = vec![image("https://example.test/declared.png")];
        g.m.plugins.panels.push(PluginPanel {
            key: key.clone(),
            plugin: "news".into(),
            def: CustomPanelDef { id: "main".into(), title: "News".into(), icon: None, width: 800.0, height: 560.0, tabs: Vec::new(), view: declared },
        });
        let pixels = || cairn_bridge::IconImage { width: 1, height: 1, rgba: Arc::new(vec![255; 4]) };
        let load = |g: &mut Gui| {
            for url in ["https://example.test/declared.png", "https://example.test/cover-7.png", "https://example.test/unused.png"] {
                g.m.icons.insert(url.into(), pixels());
            }
        };

        // Until the plugin sends a view, the declared one is drawn.
        load(&mut g);
        g.after_update();
        assert!(g.m.icons.contains_key("https://example.test/declared.png"));
        assert!(!g.m.icons.contains_key("https://example.test/unused.png"));

        // The plugin's own view replaces it, also for pictures nested in groups and named through `{data.key}`.
        let nested = PanelItem::Card {
            title: None,
            icon: None,
            children: vec![PanelItem::Row { gap: 4.0, children: vec![image("https://example.test/cover-{data.cover}.png")] }],
        };
        let data = std::collections::BTreeMap::from([("cover".to_string(), "7".to_string())]);
        g.m.plugins.live.insert(key.clone(), LiveWidget { data, view: Some(vec![nested]), view_gen: 1 });
        load(&mut g);
        g.after_update();
        assert!(g.icon("https://example.test/cover-7.png").is_some());
        assert!(!g.m.icons.contains_key("https://example.test/declared.png"));
        assert!(!g.m.icons.contains_key("https://example.test/unused.png"));
        assert_eq!(g.icon_cache.borrow().len(), 1);

        // Once no view names a picture, it goes.
        g.m.plugins.live.insert(key, LiveWidget { view: Some(Vec::new()), ..LiveWidget::default() });
        load(&mut g);
        g.after_update();
        assert!(g.m.icons.is_empty());
        assert!(g.icon_cache.borrow().is_empty());
    }
}
