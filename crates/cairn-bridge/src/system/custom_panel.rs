//! Declarative YAML models and loaders for custom panels and home widgets.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelAction {
    Launch {
        #[serde(default)]
        instance: Option<String>,
        #[serde(default)]
        server: Option<String>,
    },
    OpenUrl {
        url: String,
    },
    OpenSheet {
        sheet: String,
    },
    Copy {
        text: String,
    },
    /// Opens a panel. In a plugin's widget or panel it is one of that plugin's panels; in YAML
    /// widgets and panels it is the YAML panel with this id.
    OpenPanel {
        panel: String,
    },
    /// Runs a command of the plugin that owns the widget, with `args` passed through. Does nothing
    /// in YAML widgets.
    Command {
        command: String,
        #[serde(default)]
        args: Option<serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "element", rename_all = "snake_case")]
pub enum PanelItem {
    Banner {
        #[serde(default)]
        title: String,
        #[serde(default)]
        subtitle: Option<String>,
        #[serde(default)]
        image: Option<String>,
        #[serde(default)]
        height: Option<f32>,
        #[serde(default)]
        dim: Option<f32>,
        #[serde(default)]
        button_text: Option<String>,
        #[serde(default)]
        action: Option<PanelAction>,
    },
    Card {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        icon: Option<String>,
        #[serde(default)]
        children: Vec<PanelItem>,
    },
    Grid {
        #[serde(default = "default_columns")]
        columns: usize,
        #[serde(default = "default_gap")]
        gap: f32,
        #[serde(default)]
        children: Vec<PanelItem>,
    },
    Row {
        #[serde(default = "default_gap")]
        gap: f32,
        #[serde(default)]
        children: Vec<PanelItem>,
    },
    Column {
        #[serde(default = "default_gap")]
        gap: f32,
        #[serde(default)]
        children: Vec<PanelItem>,
    },
    Text {
        text: String,
        #[serde(default = "default_text_size")]
        size: f32,
        #[serde(default)]
        weight: Option<String>, // "bold", "normal"
        #[serde(default)]
        color: Option<String>, // "accent", "secondary", "primary"
        #[serde(default)]
        truncate: bool,
    },
    Markdown {
        content: String,
    },
    Button {
        text: String,
        #[serde(default)]
        icon: Option<String>,
        #[serde(default)]
        style: Option<String>, // "primary", "secondary"
        #[serde(default)]
        action: Option<PanelAction>,
    },
    Divider,
    Progress {
        #[serde(default)]
        value: f32,
        #[serde(default = "default_max")]
        max: f32,
        #[serde(default)]
        label: Option<String>,
        /// In plugin widgets, takes the value from this data key instead of `value`.
        #[serde(default)]
        bind: Option<String>,
    },
    /// A switch in a plugin widget. It shows the data value under `key` and runs `command` with
    /// `{ widget, key, value }` when flipped.
    Toggle {
        label: String,
        key: String,
        command: String,
    },
    /// A text field in a plugin widget. Submitting runs `command` with `{ widget, key, value }`.
    Input {
        key: String,
        command: String,
        #[serde(default)]
        placeholder: Option<String>,
        /// Text of the submit button; without one, Enter submits.
        #[serde(default)]
        button: Option<String>,
    },
    Badge {
        text: String,
        #[serde(default)]
        color: Option<String>,
    },
    Image {
        url: String,
        #[serde(default)]
        height: Option<f32>,
        #[serde(default)]
        radius: Option<f32>,
    },
    Segmented {
        options: Vec<SegmentedOption>,
        #[serde(default)]
        selected: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentedOption {
    pub label: String,
    #[serde(default)]
    pub action: Option<PanelAction>,
}

fn default_columns() -> usize {
    2
}
fn default_gap() -> f32 {
    10.0
}
fn default_text_size() -> f32 {
    13.0
}
fn default_max() -> f32 {
    100.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTabDef {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub view: Vec<PanelItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPanelDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "default_width")]
    pub width: f32,
    #[serde(default = "default_height")]
    pub height: f32,
    #[serde(default)]
    pub tabs: Vec<CustomTabDef>,
    #[serde(default)]
    pub view: Vec<PanelItem>,
}

fn default_width() -> f32 {
    800.0
}
fn default_height() -> f32 {
    560.0
}

/// Definition of a custom home widget loaded from `<data_dir>/widgets/*.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomWidgetDef {
    /// Identifier referenced as `custom:<id>` in the home layout; should remain stable across renames.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Default grid dimensions as `[columns, rows]`. The home grid is 12 columns by 6 rows.
    #[serde(default = "default_widget_size")]
    pub size: (u16, u16),
    #[serde(default)]
    pub sizes: Vec<(u16, u16)>,
    #[serde(default = "default_widget_min")]
    pub min: (u16, u16),
    #[serde(default = "default_widget_max")]
    pub max: (u16, u16),
    #[serde(default = "default_true")]
    pub heading: bool,
    #[serde(default)]
    pub view: Vec<PanelItem>,
}

fn default_widget_size() -> (u16, u16) {
    (4, 2)
}
fn default_widget_min() -> (u16, u16) {
    (2, 1)
}
fn default_widget_max() -> (u16, u16) {
    (12, 6)
}
fn default_true() -> bool {
    true
}

impl CustomWidgetDef {
    /// Loads all custom widgets from `<dir>/widgets/*.{yaml,yml}` in alphabetical order.
    /// Uses the same validation and enable state as the extension manager.
    pub fn load_all(dir: &Path) -> Vec<CustomWidgetDef> {
        let catalog = super::extensions::ExtensionCatalog::load(dir);
        catalog
            .entries
            .iter()
            .filter(|entry| catalog.enabled(entry))
            .filter_map(|entry| match &entry.content {
                Some(super::extensions::ExtensionContent::Widget(def)) => Some(def.clone()),
                _ => None,
            })
            .collect()
    }
}

impl CustomPanelDef {
    /// Loads enabled, valid custom panels from `<dir>/panels/*.{yaml,yml}`.
    pub fn load_all(dir: &Path) -> Vec<CustomPanelDef> {
        let catalog = super::extensions::ExtensionCatalog::load(dir);
        catalog
            .entries
            .iter()
            .filter(|entry| catalog.enabled(entry))
            .filter_map(|entry| match &entry.content {
                Some(super::extensions::ExtensionContent::Panel(def)) => Some(def.clone()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const FULL_YAML: &str = r#"
id: weather
name: 天气
summary: 今天的天气
icon: sun
size: [4, 2]
sizes: [[2, 2], [6, 3]]
min: [3, 2]
max: [10, 4]
heading: false
view:
  - element: text
    text: 你好
  - element: button
    text: 打开官网
    action: { type: open_url, url: "https://example.com" }
"#;

    #[test]
    fn parses_full_widget_yaml() {
        let def: CustomWidgetDef = serde_yaml::from_str(FULL_YAML).expect("完整的 YAML 应当能解析");
        assert_eq!(def.id, "weather");
        assert_eq!(def.name, "天气");
        assert_eq!(def.summary.as_deref(), Some("今天的天气"));
        assert_eq!(def.icon.as_deref(), Some("sun"));
        assert_eq!(def.size, (4, 2));
        let sizes: Vec<(u16, u16)> = vec![(2, 2), (6, 3)];
        assert_eq!(def.sizes, sizes);
        assert_eq!(def.min, (3, 2));
        assert_eq!(def.max, (10, 4));
        assert!(!def.heading);
        assert_eq!(def.view.len(), 2);
        match &def.view[0] {
            PanelItem::Text { text, .. } => assert_eq!(text, "你好"),
            other => panic!("第一项应当是文字,实际是 {other:?}"),
        }
        match &def.view[1] {
            PanelItem::Button { text, action, .. } => {
                assert_eq!(text, "打开官网");
                assert_eq!(action, &Some(PanelAction::OpenUrl { url: "https://example.com".into() }));
            }
            other => panic!("第二项应当是按钮,实际是 {other:?}"),
        }
    }

    #[test]
    fn minimal_widget_yaml_applies_defaults() {
        let def: CustomWidgetDef = serde_yaml::from_str("id: minimal\nname: 极简\n").expect("最小的 YAML 应当能解析");
        assert_eq!(def.id, "minimal");
        assert_eq!(def.name, "极简");
        assert!(def.summary.is_none());
        assert!(def.icon.is_none());
        assert_eq!(def.size, (4, 2));
        assert!(def.sizes.is_empty());
        assert_eq!(def.min, (2, 1));
        assert_eq!(def.max, (12, 6));
        assert!(def.heading);
        assert!(def.view.is_empty());
    }

    #[test]
    fn load_all_skips_bad_and_duplicate_files() {
        let root = std::env::temp_dir().join(format!("cairn-widgets-test-{}-load", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let widgets = root.join("widgets");
        fs::create_dir_all(&widgets).expect("创建临时目录");
        fs::write(widgets.join("b.yaml"), "id: b\nname: 乙\n").expect("写入 b.yaml");
        fs::write(widgets.join("a.yml"), "id: a\nname: 甲\n").expect("写入 a.yml");
        fs::write(widgets.join("c.yaml"), "id: [c\nname: 格式错误\n").expect("写入 c.yaml");
        fs::write(widgets.join("d.yaml"), "id: a\nname: 重复\n").expect("写入 d.yaml");
        fs::write(widgets.join("e.yaml"), "id: \"  \"\nname: 空编号\n").expect("写入 e.yaml");
        fs::write(widgets.join("note.txt"), "id: n\nname: 笔记\n").expect("写入 note.txt");

        let list = CustomWidgetDef::load_all(&root);
        let ids: Vec<&str> = list.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["b"]);
        assert_eq!(list[0].name, "乙");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn load_all_without_widgets_dir_is_empty() {
        let root = std::env::temp_dir().join(format!("cairn-widgets-test-{}-none", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("创建临时目录");

        assert!(CustomWidgetDef::load_all(&root).is_empty());
        assert!(!root.join("widgets").exists());

        let _ = fs::remove_dir_all(&root);
    }
}
