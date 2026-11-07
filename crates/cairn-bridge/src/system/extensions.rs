//! Local declarative extensions, their diagnostics and persistent enable state.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::custom_panel::{CustomPanelDef, CustomWidgetDef, PanelAction, PanelItem};
use crate::{Bridge, Model, ToastKind};

const STATE_FILE: &str = "extensions.json";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionKind {
    Widget,
    Panel,
}

#[derive(Debug, Clone)]
pub enum ExtensionContent {
    Widget(CustomWidgetDef),
    Panel(CustomPanelDef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionIssue {
    Invalid(String),
    EmptyId,
    Duplicate(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtensionPermission {
    Launch,
    OpenUrl(String),
    OpenSheet(String),
    Copy,
}

#[derive(Debug, Clone)]
pub struct ExtensionEntry {
    pub key: String,
    pub source: PathBuf,
    pub kind: ExtensionKind,
    pub id: Option<String>,
    pub name: String,
    pub content: Option<ExtensionContent>,
    pub issue: Option<ExtensionIssue>,
}

impl ExtensionEntry {
    fn state_key(&self) -> String {
        let kind = match self.kind {
            ExtensionKind::Widget => "widget",
            ExtensionKind::Panel => "panel",
        };
        format!("{kind}:{}", self.id.as_deref().unwrap_or_default())
    }

    pub fn permissions(&self) -> Vec<ExtensionPermission> {
        fn visit(items: &[PanelItem], found: &mut BTreeSet<ExtensionPermission>) {
            for item in items {
                match item {
                    PanelItem::Button { action, .. } | PanelItem::Banner { action, .. } => {
                        if let Some(action) = action {
                            found.insert(match action {
                                PanelAction::Launch { .. } => ExtensionPermission::Launch,
                                PanelAction::OpenUrl { url } => ExtensionPermission::OpenUrl(url.clone()),
                                PanelAction::OpenSheet { sheet } => ExtensionPermission::OpenSheet(sheet.clone()),
                                PanelAction::OpenPanel { panel } => ExtensionPermission::OpenSheet(format!("panel:{panel}")),
                                PanelAction::Copy { .. } => ExtensionPermission::Copy,
                                // Commands only run in plugin widgets, which have their own permissions.
                                PanelAction::Command { .. } => continue,
                            });
                        }
                    }
                    PanelItem::Segmented { options, .. } => {
                        for opt in options {
                            if let Some(action) = &opt.action {
                                found.insert(match action {
                                    PanelAction::Launch { .. } => ExtensionPermission::Launch,
                                    PanelAction::OpenUrl { url } => ExtensionPermission::OpenUrl(url.clone()),
                                    PanelAction::OpenSheet { sheet } => ExtensionPermission::OpenSheet(sheet.clone()),
                                    PanelAction::OpenPanel { panel } => ExtensionPermission::OpenSheet(format!("panel:{panel}")),
                                    PanelAction::Copy { .. } => ExtensionPermission::Copy,
                                    PanelAction::Command { .. } => continue,
                                });
                            }
                        }
                    }
                    PanelItem::Card { children, .. }
                    | PanelItem::Grid { children, .. }
                    | PanelItem::Row { children, .. }
                    | PanelItem::Column { children, .. } => visit(children, found),
                    _ => {}
                }
            }
        }
        let mut found = BTreeSet::new();
        match &self.content {
            Some(ExtensionContent::Widget(def)) => visit(&def.view, &mut found),
            Some(ExtensionContent::Panel(def)) => {
                visit(&def.view, &mut found);
                for tab in &def.tabs {
                    visit(&tab.view, &mut found);
                }
            }
            None => {}
        }
        found.into_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionState {
    version: u32,
    #[serde(default)]
    disabled: BTreeSet<String>,
}

impl Default for ExtensionState {
    fn default() -> Self {
        Self { version: 1, disabled: BTreeSet::new() }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionCatalog {
    pub entries: Vec<ExtensionEntry>,
    pub error: Option<String>,
    pub busy: bool,
    state: ExtensionState,
}

impl ExtensionCatalog {
    pub fn load(dir: &Path) -> Self {
        let mut catalog = Self::default();
        match read_state(dir) {
            Ok(state) => catalog.state = state,
            Err(error) => catalog.error = Some(error),
        }
        match scan(dir) {
            Ok(entries) => catalog.entries = entries,
            Err(error) => catalog.error = Some(error),
        }
        catalog
    }

    pub fn enabled(&self, entry: &ExtensionEntry) -> bool {
        self.error.is_none() && entry.issue.is_none() && entry.content.is_some() && !self.state.disabled.contains(&entry.state_key())
    }

    pub(crate) fn apply(self, model: &mut Model) {
        model.widgets = self
            .entries
            .iter()
            .filter(|entry| self.enabled(entry))
            .filter_map(|entry| match &entry.content {
                Some(ExtensionContent::Widget(def)) => Some(def.clone()),
                _ => None,
            })
            .collect();
        model.panels = self
            .entries
            .iter()
            .filter(|entry| self.enabled(entry))
            .filter_map(|entry| match &entry.content {
                Some(ExtensionContent::Panel(def)) => Some(def.clone()),
                _ => None,
            })
            .collect();
        model.extensions = self;
    }
}

fn read_state(dir: &Path) -> Result<ExtensionState, String> {
    let path = dir.join(STATE_FILE);
    let bytes = match read_file(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ExtensionState::default()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let state: ExtensionState = serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if state.version != 1 {
        return Err(format!("{}: unsupported version {}", path.display(), state.version));
    }
    Ok(state)
}

fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Configuration exceeds 1 MiB"));
    }
    Ok(bytes)
}

fn scan(dir: &Path) -> Result<Vec<ExtensionEntry>, String> {
    let mut entries = Vec::new();
    for (folder, kind) in [("widgets", ExtensionKind::Widget), ("panels", ExtensionKind::Panel)] {
        let path = dir.join(folder);
        let files = match fs::read_dir(&path) {
            Ok(files) => files,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let mut files: Vec<_> = files
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml"))
            .collect();
        files.sort();
        let mut seen = BTreeMap::<String, PathBuf>::new();
        for source in files {
            let mut entry = ExtensionEntry {
                key: format!("{folder}/{}", source.file_name().unwrap_or_default().to_string_lossy()),
                name: source.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                source,
                kind,
                id: None,
                content: None,
                issue: None,
            };
            match read_definition(&entry.source, kind) {
                Ok(content) => {
                    let (id, name) = match &content {
                        ExtensionContent::Widget(def) => (&def.id, &def.name),
                        ExtensionContent::Panel(def) => (&def.id, &def.title),
                    };
                    entry.name = name.clone();
                    entry.id = Some(id.clone());
                    if id.is_empty() {
                        entry.issue = Some(ExtensionIssue::EmptyId);
                    } else if let Some(previous) = seen.insert(id.clone(), entry.source.clone()) {
                        entry.issue = Some(ExtensionIssue::Duplicate(previous));
                    }
                    entry.content = Some(content);
                }
                Err(error) => entry.issue = Some(ExtensionIssue::Invalid(error)),
            }
            entries.push(entry);
        }
        // Conflicting IDs must never execute whichever file happens to sort first.
        let duplicates: BTreeMap<_, _> = entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .filter_map(|entry| match (&entry.id, &entry.issue) {
                (Some(id), Some(ExtensionIssue::Duplicate(_))) => Some((id.clone(), entry.source.clone())),
                _ => None,
            })
            .collect();
        for entry in entries.iter_mut().filter(|entry| entry.kind == kind) {
            if entry.issue.is_none()
                && let Some(other) = entry.id.as_ref().and_then(|id| duplicates.get(id))
            {
                entry.issue = Some(ExtensionIssue::Duplicate(other.clone()));
            }
        }
    }
    Ok(entries)
}

fn read_definition(path: &Path, kind: ExtensionKind) -> Result<ExtensionContent, String> {
    let yaml = String::from_utf8(read_file(path).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let content = match kind {
        ExtensionKind::Widget => {
            let mut def: CustomWidgetDef = serde_yaml::from_str(&yaml).map_err(|error| error.to_string())?;
            def.id = def.id.trim().into();
            validate_items(&def.view, 0, &mut 0)?;
            ExtensionContent::Widget(def)
        }
        ExtensionKind::Panel => {
            let mut def: CustomPanelDef = serde_yaml::from_str(&yaml).map_err(|error| error.to_string())?;
            def.id = def.id.trim().into();
            if !def.width.is_finite() || !def.height.is_finite() || def.width <= 0.0 || def.height <= 0.0 {
                return Err("Panel dimensions must be finite and positive".into());
            }
            let mut count = 0;
            validate_items(&def.view, 0, &mut count)?;
            for tab in &def.tabs {
                validate_items(&tab.view, 0, &mut count)?;
            }
            ExtensionContent::Panel(def)
        }
    };
    Ok(content)
}

pub(crate) fn validate_items(items: &[PanelItem], depth: usize, count: &mut usize) -> Result<(), String> {
    *count += items.len();
    if depth > 32 || *count > 1024 {
        return Err("UI exceeds 32 nesting levels or 1024 elements".into());
    }
    for item in items {
        let valid = match item {
            PanelItem::Text { size, .. } => size.is_finite() && *size > 0.0 && *size <= 256.0,
            PanelItem::Progress { value, max, .. } => value.is_finite() && max.is_finite() && *max > 0.0,
            PanelItem::Toggle { key, command, .. } | PanelItem::Input { key, command, .. } => !key.is_empty() && !command.is_empty(),
            PanelItem::Banner { height, dim, .. } => {
                height.is_none_or(|h| h.is_finite() && h > 0.0 && h <= 2048.0) && dim.is_none_or(|d| d.is_finite() && (0.0..=1.0).contains(&d))
            }
            PanelItem::Grid { columns, gap, .. } => (1..=12).contains(columns) && gap.is_finite() && (0.0..=256.0).contains(gap),
            PanelItem::Row { gap, .. } | PanelItem::Column { gap, .. } => gap.is_finite() && (0.0..=256.0).contains(gap),
            PanelItem::Image { height, radius, .. } => {
                height.is_none_or(|h| h.is_finite() && h > 0.0 && h <= 2048.0) && radius.is_none_or(|r| r.is_finite() && r >= 0.0 && r <= 128.0)
            }
            PanelItem::Segmented { options, selected } => {
                !options.is_empty() && options.len() <= 16 && *selected < options.len()
            }
            _ => true,
        };
        if !valid {
            return Err("UI dimensions and progress values are outside the supported range".into());
        }
        match item {
            PanelItem::Card { children, .. } | PanelItem::Grid { children, .. } | PanelItem::Row { children, .. } | PanelItem::Column { children, .. } => {
                validate_items(children, depth + 1, count)?
            }
            _ => {}
        }
    }
    Ok(())
}

impl Bridge {
    pub fn reload_panels(&self, model: &mut Model) {
        if model.extensions.busy {
            return;
        }
        let Some(dir) = model.data_dir.clone() else { return };
        model.extensions.busy = true;
        let bridge = self.clone();
        self.inner.rt.spawn(async move {
            let loaded = tokio::task::spawn_blocking(move || ExtensionCatalog::load(&dir)).await;
            bridge.post(move |bridge, model| match loaded {
                Ok(mut catalog) => {
                    for entry in &mut catalog.entries {
                        if entry.id.is_none()
                            && let Some(previous) = model.extensions.entries.iter().find(|old| old.key == entry.key)
                        {
                            entry.id = previous.id.clone();
                            entry.name = previous.name.clone();
                        }
                    }
                    catalog.apply(model);
                }
                Err(error) => {
                    model.extensions.busy = false;
                    bridge.toast(model, ToastKind::Error, error.to_string());
                }
            });
        });
    }

    pub fn set_extension_enabled(&self, model: &mut Model, key: &str, enabled: bool) {
        let catalog = &model.extensions;
        if catalog.busy || catalog.error.is_some() {
            return;
        }
        let Some(entry) = catalog.entries.iter().find(|entry| entry.key == key && entry.issue.is_none()) else { return };
        let state_key = entry.state_key();
        let Some(dir) = model.data_dir.clone() else { return };
        let previous = catalog.state.clone();
        let mut next = previous.clone();
        if enabled {
            next.disabled.remove(&state_key);
        } else {
            next.disabled.insert(state_key);
        }
        if next == previous {
            return;
        }
        model.extensions.busy = true;
        let bridge = self.clone();
        let save = self.inner.rt.spawn(async move {
            let saved = save_state(&dir, &previous, &next).await;
            bridge.post(move |bridge, model| {
                model.extensions.busy = false;
                match saved {
                    Ok(()) => {
                        let mut catalog = model.extensions.clone();
                        catalog.state = next;
                        catalog.apply(model);
                    }
                    Err(error) => bridge.toast(model, ToastKind::Error, cairn_i18n::tr!("extensions.save_failed", error = error)),
                }
            });
        });
        *self.inner.extension_save.lock().unwrap() = Some(save);
    }
}

async fn save_state(dir: &Path, previous: &ExtensionState, next: &ExtensionState) -> Result<(), String> {
    let read_dir = dir.to_owned();
    let current = tokio::task::spawn_blocking(move || read_state(&read_dir)).await.map_err(|error| error.to_string())??;
    if &current != previous {
        return Err("Enable state changed on disk; reload before saving".into());
    }
    let bytes = serde_json::to_vec_pretty(next).map_err(|error| error.to_string())?;
    cairn_core::util::fsx::write_atomic(&dir.join(STATE_FILE), &bytes).await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("cairn-extensions-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn catalog_keeps_invalid_files_and_blocks_all_conflicting_ids() {
        let dir = TestDir::new();
        dir.write("widgets/a.yaml", "id: ' same '\nname: First\n");
        dir.write("widgets/b.yml", "id: same\nname: Second\n");
        dir.write("widgets/c.yaml", "id: [broken\n");
        dir.write("widgets/d.yaml", "id: '  '\nname: Empty\n");
        dir.write("widgets/e.yaml", "id: valid\nname: Valid\n");
        dir.write("widgets/ignored.txt", "invalid");
        dir.write("panels/a.yaml", "id: same\ntitle: Panel\n");
        let catalog = ExtensionCatalog::load(&dir.0);
        assert!(catalog.error.is_none());
        assert_eq!(catalog.entries.len(), 6);
        assert!(catalog.entries[..2].iter().all(|entry| matches!(entry.issue, Some(ExtensionIssue::Duplicate(_))) && !catalog.enabled(entry)));
        assert!(matches!(catalog.entries[2].issue, Some(ExtensionIssue::Invalid(_))));
        assert_eq!(catalog.entries[3].issue, Some(ExtensionIssue::EmptyId));
        assert!(catalog.enabled(&catalog.entries[4]));
        assert!(catalog.enabled(&catalog.entries[5]));
        assert_eq!(catalog.entries[4].source, dir.0.join("widgets/e.yaml"));
    }

    #[tokio::test]
    async fn enable_state_survives_reload_without_changing_yaml_or_layout() {
        let dir = TestDir::new();
        let yaml = "id: clock\nname: Clock\n";
        dir.write("widgets/clock.yaml", yaml);
        let layout = crate::BoardLayout {
            version: 1,
            items: vec![crate::BoardItem {
                id: "custom:clock-1".into(),
                kind: "custom:clock".into(),
                x: 4,
                y: 2,
                w: 4,
                h: 2,
                options: BTreeMap::from([("custom-setting".into(), "keep".into())]),
            }],
            hide_empty_hint: true,
        };
        let mut model = Model { board: Some(layout.clone()), ..Model::default() };
        let catalog = ExtensionCatalog::load(&dir.0);
        let previous = catalog.state.clone();
        let mut disabled = previous.clone();
        disabled.disabled.insert("widget:clock".into());
        save_state(&dir.0, &previous, &disabled).await.unwrap();
        ExtensionCatalog::load(&dir.0).apply(&mut model);
        assert!(model.widgets.is_empty());
        assert_eq!(model.board, Some(layout.clone()));
        fs::rename(dir.0.join("widgets/clock.yaml"), dir.0.join("widgets/renamed.yaml")).unwrap();
        let renamed = ExtensionCatalog::load(&dir.0);
        assert!(!renamed.enabled(&renamed.entries[0]));
        save_state(&dir.0, &disabled, &previous).await.unwrap();
        ExtensionCatalog::load(&dir.0).apply(&mut model);
        assert_eq!(model.widgets[0].id, "clock");
        assert_eq!(model.board, Some(layout));
        assert_eq!(fs::read_to_string(dir.0.join("widgets/renamed.yaml")).unwrap(), yaml);
    }

    #[tokio::test]
    async fn corrupt_or_changed_state_is_not_overwritten_or_silently_enabled() {
        let dir = TestDir::new();
        dir.write("widgets/clock.yaml", "id: clock\nname: Clock\n");
        let previous = ExtensionState::default();
        let mut next = previous.clone();
        next.disabled.insert("widget:clock".into());
        dir.write(STATE_FILE, "broken");
        let catalog = ExtensionCatalog::load(&dir.0);
        assert!(catalog.error.is_some());
        assert!(!catalog.enabled(&catalog.entries[0]));
        assert!(save_state(&dir.0, &previous, &next).await.is_err());
        assert_eq!(fs::read_to_string(dir.0.join(STATE_FILE)).unwrap(), "broken");
        dir.write(STATE_FILE, &serde_json::to_string(&next).unwrap());
        assert!(save_state(&dir.0, &previous, &next).await.is_err());
        assert_eq!(read_state(&dir.0).unwrap(), next);
        fs::create_dir(dir.0.join("extensions.json.tmp")).unwrap();
        assert!(save_state(&dir.0, &next, &previous).await.is_err());
        assert_eq!(read_state(&dir.0).unwrap(), next);
        dir.write(STATE_FILE, "{\"version\":99}");
        assert!(ExtensionCatalog::load(&dir.0).error.is_some());
    }

    #[test]
    fn permissions_include_nested_actions_and_all_panel_tabs() {
        let dir = TestDir::new();
        dir.write("panels/actions.yaml", "id: actions\ntitle: Actions\nview:\n  - element: column\n    children:\n      - element: button\n        text: Start\n        action: {type: launch}\ntabs:\n  - id: links\n    label: Links\n    view:\n      - element: banner\n        action: {type: open_url, url: 'https://example.com'}\n      - element: button\n        text: Copy\n        action: {type: copy, text: value}\n      - element: button\n        text: Settings\n        action: {type: open_sheet, sheet: settings}\n");
        let catalog = ExtensionCatalog::load(&dir.0);
        assert!(catalog.enabled(&catalog.entries[0]));
        assert_eq!(
            catalog.entries[0].permissions(),
            vec![
                ExtensionPermission::Launch,
                ExtensionPermission::OpenUrl("https://example.com".into()),
                ExtensionPermission::OpenSheet("settings".into()),
                ExtensionPermission::Copy
            ]
        );
    }

    #[test]
    fn segmented_and_image_validation_and_permissions() {
        let dir = TestDir::new();
        dir.write(
            "panels/custom.yaml",
            "id: custom\ntitle: Custom\nview:\n  - element: image\n    url: 'https://example.com/a.png'\n    height: 160\n    radius: 8\n  - element: segmented\n    selected: 0\n    options:\n      - label: Tab1\n        action: {type: open_url, url: 'https://tab1.com'}\n      - label: Tab2\n",
        );
        let catalog = ExtensionCatalog::load(&dir.0);
        assert!(catalog.enabled(&catalog.entries[0]));
        assert_eq!(
            catalog.entries[0].permissions(),
            vec![ExtensionPermission::OpenUrl("https://tab1.com".into())]
        );

        let mut count = 0;
        let invalid_img = vec![PanelItem::Image { url: "a".into(), height: Some(-10.0), radius: None }];
        assert!(validate_items(&invalid_img, 0, &mut count).is_err());

        count = 0;
        let invalid_seg = vec![PanelItem::Segmented { options: Vec::new(), selected: 0 }];
        assert!(validate_items(&invalid_seg, 0, &mut count).is_err());
    }

    #[test]
    fn unsafe_dimensions_and_oversized_files_stay_visible_as_errors() {
        let dir = TestDir::new();
        dir.write("widgets/progress.yaml", "id: invalid\nname: Invalid\nview:\n  - element: progress\n    value: .nan\n    max: 0\n");
        dir.write("widgets/large.yaml", &" ".repeat(MAX_CONFIG_BYTES as usize + 1));
        let catalog = ExtensionCatalog::load(&dir.0);
        assert_eq!(catalog.entries.len(), 2);
        assert!(catalog.entries.iter().all(|entry| matches!(entry.issue, Some(ExtensionIssue::Invalid(_))) && !catalog.enabled(entry)));
        dir.write(STATE_FILE, &" ".repeat(MAX_CONFIG_BYTES as usize + 1));
        assert!(ExtensionCatalog::load(&dir.0).error.is_some());
    }
}
