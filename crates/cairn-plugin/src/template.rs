//! A starting point for new plugins: a manifest with a widget, a command and a setting, an entry
//! module, and the API type definitions with an editor config that checks the JavaScript.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::manifest::{self, Manifest};

/// TypeScript definitions of the plugin API.
pub const TYPES: &str = include_str!("../sdk/cairn.d.ts");

const MAIN: &str = r#"/// <reference path="./cairn.d.ts" />
// activate() runs when the plugin is enabled. Save this file while the launcher runs and the plugin
// reloads. See the plugin guide for the whole API.

/** @param {Cairn} cairn */
export function activate(cairn) {
  const show = () =>
    cairn.widgets.setData("hello", {
      greeting: cairn.settings.get("greeting", "Hello"),
      time: new Date().toLocaleTimeString(),
    });
  show();
  setInterval(show, 1000);
  cairn.settings.onChange(show);

  // A returned string is shown as a notification.
  cairn.commands.register("say-hello", () => `${cairn.settings.get("greeting", "Hello")} from ${cairn.plugin.name}!`);
}

export function deactivate() {}
"#;

const JSCONFIG: &str = r#"{
  "compilerOptions": {
    "checkJs": true,
    "module": "ES2022",
    "target": "ES2022",
    "lib": ["ES2022"]
  },
  "include": ["**/*.js", "cairn.d.ts"]
}
"#;

/// The first free id of the form `base`, `base-2`, `base-3`… in the plugins folder.
pub fn unused_id(data_dir: &Path, base: &str) -> String {
    let taken: Vec<String> = manifest::scan(data_dir).unwrap_or_default().iter().filter_map(|e| e.id().map(String::from)).collect();
    let plugins = data_dir.join(manifest::PLUGINS_DIR);
    (1..)
        .map(|n| if n == 1 { base.to_string() } else { format!("{base}-{n}") })
        .find(|id| !taken.contains(id) && !plugins.join(id).exists())
        .expect("some suffix is free")
}

/// Creates `plugins/<id>/` with a working plugin. It starts disabled, like any installed plugin.
pub fn create(data_dir: &Path, id: &str, name: &str) -> Result<PathBuf, String> {
    if !manifest::valid_plugin_id(id) {
        return Err(format!("Invalid plugin id '{id}': use 1-64 lowercase letters, digits, '.', '_' or '-'"));
    }
    let dir = data_dir.join(manifest::PLUGINS_DIR).join(id);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let manifest = json!({
        "id": id,
        "name": name,
        "version": "0.1.0",
        "description": "Describe what your plugin does.",
        "api": manifest::API_VERSION,
        "main": "main.js",
        "permissions": [],
        "settings": [
            { "id": "greeting", "title": "Greeting", "type": "string", "default": "Hello" }
        ],
        "widgets": [
            {
                "id": "hello",
                "name": name,
                "icon": "puzzle",
                "size": [4, 2],
                "view": [
                    { "element": "text", "text": "{data.greeting}", "size": 22, "weight": "bold" },
                    { "element": "text", "text": "{data.time}", "color": "secondary" },
                    { "element": "button", "text": "Say hello", "action": { "type": "command", "command": "say-hello" } }
                ]
            }
        ],
        "commands": [{ "id": "say-hello", "title": "Say hello" }]
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    // The template must stay valid as the manifest format evolves.
    Manifest::parse(text.as_bytes())?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (file, content) in [("plugin.json", text.as_str()), ("main.js", MAIN), ("cairn.d.ts", TYPES), ("jsconfig.json", JSCONFIG)] {
        fs::write(dir.join(file), content).map_err(|e| format!("{}: {e}", dir.join(file).display()))?;
    }
    Ok(dir)
}

/// Writes the current type definitions into an existing plugin folder.
pub fn write_types(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("cairn.d.ts");
    fs::write(&path, TYPES).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_a_valid_package_and_ids_do_not_collide() {
        let data = std::env::temp_dir().join(format!("cairn-plugin-template-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        assert_eq!(unused_id(&data, "my-plugin"), "my-plugin");
        let dir = create(&data, "my-plugin", "My plugin").unwrap();
        assert!(["plugin.json", "main.js", "cairn.d.ts", "jsconfig.json"].iter().all(|f| dir.join(f).is_file()));
        let entries = manifest::scan(&data).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].issue.is_none(), "{:?}", entries[0].issue);
        assert_eq!(unused_id(&data, "my-plugin"), "my-plugin-2");
        assert!(create(&data, "my-plugin", "Again").is_err());
        assert!(create(&data, "Bad Id", "Bad").is_err());
        fs::remove_dir_all(&data).unwrap();
    }
}
