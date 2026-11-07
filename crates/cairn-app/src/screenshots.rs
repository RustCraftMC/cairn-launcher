//! Layout tests for the library and import sheets, using fake fixture data.
//! The `export_site_*` tests at the end write the website screenshots. They are ignored by default:
//! `cargo test -p cairn-app export_site -- --ignored --nocapture`. `CAIRN_SITE_SHOTS_DIR` picks the folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cairn_bridge::*;
use cairn_ui::testing::Headless;

use crate::gui::{Gui, Paged, SettingsTab, Sheet};

fn inst(id: &str, name: &str, mc: &str, loaders: Vec<LoaderInfo>, isolated: bool) -> Instance {
    let root = std::path::PathBuf::from(r"D:\Games\.minecraft");
    Instance {
        id: format!("当前文件夹/{id}"),
        name: name.into(),
        layout: Layout::Standard,
        root: root.clone(),
        version_id: id.into(),
        game_dir: if isolated { root.join("versions").join(id) } else { root },
        settings: InstanceSettings::default(),
        mc_version: Some(mc.into()),
        loaders,
        release_time: None,
        problem: None,
    }
}

fn model(m: &mut Model, light: bool) {
    m.status = Status::Ready;
    m.config.setup_completed = true;
    m.config.eula_accepted = true;
    m.appearance.mode = if light { ThemeMode::Light } else { ThemeMode::Dark };
    let li = |kind, v: &str| LoaderInfo { kind, version: v.into() };
    let list = vec![
        inst("1.20.1-Fabric", "1.20.1-Fabric", "1.20.1", vec![li(LoaderKind::Fabric, "0.16.14")], true),
        inst("1.20.1", "1.20.1", "1.20.1", vec![], false),
        inst("Forge 整合包", "我的生存服整合包", "1.20.1", vec![li(LoaderKind::Forge, "47.2.18"), li(LoaderKind::OptiFine, "HD_U_I6")], true),
        inst("1.21.4", "1.21.4", "1.21.4", vec![], false),
    ];
    m.selected = Some(list[0].id.clone());
    m.instances = Remote::Ready(list);
    m.accounts = Remote::Ready(vec![
        AccountInfo {
            id: "a".into(),
            kind: AccountKind::Offline,
            name: "Steve".into(),
            uuid: "0".into(),
            skin_url: None,
            selected: true,
            needs_refresh: false,
            yggdrasil_server: None,
        },
        AccountInfo {
            id: "b".into(),
            kind: AccountKind::Microsoft,
            name: "Alex_2026".into(),
            uuid: "1".into(),
            skin_url: None,
            selected: false,
            needs_refresh: false,
            yggdrasil_server: None,
        },
    ]);
    let entry = |id: &str, kind| ManifestEntry {
        id: id.into(),
        kind,
        url: String::new(),
        time: String::new(),
        release_time: "2026-09-01T00:00:00+00:00".into(),
        sha1: None,
    };
    m.manifest = Remote::Ready(
        ["26.3", "26.2", "26.1", "1.21.4", "1.21.1", "1.20.6", "1.20.4", "1.20.1", "1.19.4", "1.18.2", "1.16.5", "1.12.2"]
            .iter()
            .map(|v| entry(v, VersionType::Release))
            .collect(),
    );
    m.latest_release = Some("26.3".into());
    let java = |v: &str, major, vendor: &str, home: &str, managed| {
        let home = std::path::PathBuf::from(home);
        JavaInstall {
            path: home.join(r"bin\java.exe"),
            home,
            version: v.into(),
            major,
            minor: 0,
            patch: 0,
            arch: "x64".into(),
            vendor: Some(vendor.into()),
            is_jdk: false,
            managed,
        }
    };
    m.javas = Remote::Ready(vec![
        java("21.0.5", 21, "Eclipse Adoptium", r"C:\Program Files\Eclipse Adoptium\jre-21", false),
        java("17.0.13", 17, "Azul Zulu", r"D:\Cairn\runtime\java-17", true),
        java("17.0.9", 17, "Oracle", r"C:\Program Files\Java\jdk-17", false),
    ]);
    m.hitokoto = Some(Hitokoto { text: "即使身处矿坑深处，也不要忘记仰望星空。".into(), from: None });
}

#[test]
fn library_statuses_preserve_tabs_and_neighboring_controls() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
        cairn_i18n::with_language(language, || {
            let sink: Sink = Arc::new(|_| true);
            let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
            model(&mut g.m, false);
            g.m.content.instance = g.m.selected.clone();
            g.ui.search = "fixture".into();
            g.m.online.query = "fixture".into();
            g.m.content.items = Remote::Ready(vec![ContentItem {
                path: "fixture.jar".into(),
                file_name: "fixture.jar".into(),
                title: "Local fixture".into(),
                version: None,
                description: None,
                enabled: true,
                is_dir: false,
                size: 0,
                modified: 0,
                icon_file: None,
                zh_name: None,
                wiki_url: None,
            }]);
            g.m.online.results = Remote::Ready(vec![OnlineItem {
                source: OnlineSource::Modrinth,
                project_id: "fixture".into(),
                title: "Online fixture".into(),
                description: "Description".into(),
                author: "Author".into(),
                downloads: 100,
                icon_url: None,
                state: OnlineState::Available,
                zh_name: None,
                zh_summary: None,
                wiki_url: None,
                project_url: None,
            }]);
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
            let mut app = Headless::with_theme(g, |g: &Gui| crate::views::sheet(g, Sheet::Library), 800.0, 560.0, 1.0, theme, text);
            let tabs: Vec<_> = app.find_role(cairn_ui::Role::Tab).iter().map(|tab| tab.bounds).collect();
            let open = app.find(cairn_i18n::tr!("library.open_dir")).unwrap().bounds;
            let local = app.find("Local fixture").unwrap().bounds;
            let online = app.find("Online fixture").unwrap().bounds;
            let downloads = app.find(&cairn_i18n::tr!("library.downloads_count", count = "100")).unwrap().bounds;
            for state in [OnlineState::Installing(1), OnlineState::Installed, OnlineState::Failed("Failed".into())] {
                app.update(move |g| {
                    if let Remote::Ready(items) = &mut g.m.online.results {
                        items[0].state = state;
                    }
                    for kind in ContentKind::ALL {
                        g.m.content.counts.insert(kind, 10000);
                    }
                    if let Remote::Ready(items) = &mut g.m.content.items {
                        items[0].enabled = false;
                    }
                    g.m.content.busy.insert("fixture.jar".into());
                    g.m.content.updates = Remote::Loading;
                    g.ui.drop_hover = true;
                });
                assert_eq!(app.find_role(cairn_ui::Role::Tab).iter().map(|tab| tab.bounds).collect::<Vec<_>>(), tabs);
                assert_eq!(app.find(cairn_i18n::tr!("library.open_dir")).unwrap().bounds, open);
                assert_eq!(app.find("Local fixture").unwrap().bounds, local);
                assert_eq!(app.find("Online fixture").unwrap().bounds, online);
                assert_eq!(app.find(&cairn_i18n::tr!("library.downloads_count", count = "100")).unwrap().bounds, downloads);
            }
        });
    }
}

#[test]
fn import_candidates_use_instance_icon_rules() {
    use LoaderKind::*;
    use cairn_icons::VersionIcon::{self, Anvil, Cobblestone, CommandBlock, Egg, GoldBlock, Grass, GrassPath, Pack, RedstoneBlock};

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    for (id, mc, loaders, chosen, broken, expected) in [
        ("1.21.10", None, vec![], None, false, Grass),
        ("25w42a", None, vec![], None, false, CommandBlock),
        ("26.4-snapshot-6", None, vec![], None, false, CommandBlock),
        ("renamed", Some("25w42a"), vec![], None, false, CommandBlock),
        ("b1.7.3", None, vec![], None, false, Cobblestone),
        ("24w14potato", None, vec![], None, false, GoldBlock),
        ("Forge", Some("1.20.1"), vec![Forge, OptiFine], None, false, Anvil),
        ("NeoForge", Some("1.21.1"), vec![NeoForge], None, false, VersionIcon::NeoForge),
        ("Fabric", Some("1.20.1"), vec![Fabric, OptiFine], None, false, VersionIcon::Fabric),
        ("Quilt", Some("1.20.1"), vec![Quilt], None, false, VersionIcon::Quilt),
        ("LiteLoader", Some("1.12.2"), vec![LiteLoader], None, false, Egg),
        ("OptiFine", Some("1.20.1"), vec![OptiFine], None, false, GrassPath),
        ("custom", Some("1.20.1"), vec![Forge], Some("pack"), true, Pack),
        ("unknown-icon", Some("1.20.1"), vec![Forge], Some("unknown"), false, Anvil),
        ("broken", None, vec![Fabric], None, true, RedstoneBlock),
    ] {
        let loaders = loaders.into_iter().map(|kind| LoaderInfo { kind, version: "fixture".into() }).collect();
        let mut i = inst(id, id, mc.unwrap_or(id), loaders, false);
        i.mc_version = mc.map(str::to_string);
        i.settings.icon = chosen.map(str::to_string);
        i.problem = broken.then(|| "Invalid version".into());
        let sink: Sink = Arc::new(|_| true);
        let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        model(&mut g.m, false);
        assert_eq!(crate::views::instance_icon(&g.m, &i), expected, "{id}");
        g.m.import_candidates = Remote::Ready(vec![ImportCandidate {
            id: id.into(),
            game_root: i.root,
            root_name: "fixture".into(),
            source: SourceLauncher::Vanilla,
            mc_version: i.mc_version,
            loaders: i.loaders,
            problem: i.problem,
            settings: i.settings,
            badges: vec![],
        }]);
        let theme = crate::style::theme(&g.m.appearance, None, false);
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut app = Headless::with_theme(g, |g: &Gui| crate::views::sheet(g, Sheet::InitialSetup), 800.0, 560.0, 1.0, theme, text);
        app.render();
        let icon = app.find(expected.label()).unwrap_or_else(|| panic!("Missing {expected:?} for {id}"));
        assert_eq!((icon.bounds.width, icon.bounds.height), (36.0, 36.0));
    }
}

#[test]
fn import_selection_keeps_action_bounds_stable() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
        cairn_i18n::with_language(language, || {
            let sink: Sink = Arc::new(|_| true);
            let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
            model(&mut g.m, false);
            g.m.import_candidates = Remote::Ready(vec![ImportCandidate {
                id: "1.20.1".into(),
                game_root: "D:/fixture".into(),
                root_name: "fixture".into(),
                source: SourceLauncher::Vanilla,
                mc_version: Some("1.20.1".into()),
                loaders: Vec::new(),
                problem: None,
                settings: InstanceSettings::default(),
                badges: Vec::new(),
            }]);
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
            let mut app = Headless::with_theme(g, |g: &Gui| crate::views::sheet(g, Sheet::InitialSetup), 800.0, 560.0, 1.0, theme, text);
            let import = app.find(&cairn_i18n::tr!("setup.import_selected_count", count = 1)).unwrap().bounds;
            let select = app.find(cairn_i18n::tr!("setup.deselect_all")).unwrap().bounds;
            assert!(import.right() <= 800.0 && select.right() < import.x);
            app.update(|g| g.deselect_all_candidates());
            // With nothing picked the accent fill moves from the import button to the skip button. The import button
            // gets a little shorter and the bolder skip label pushes the left side of the action bar a few pixels to
            // the right, so only the import button's position and the buttons' widths stay exact.
            let emptied = app.find(&cairn_i18n::tr!("setup.import_selected_count", count = 0)).unwrap().bounds;
            assert_eq!((emptied.x, emptied.y, emptied.width), (import.x, import.y, import.width));
            assert_eq!(app.find(cairn_i18n::tr!("setup.select_all")).unwrap().bounds.width, select.width);
        });
    }
}

// Website screenshots. Every name, number and account here is made up.
// Output: `{scene}.{lang}.{mode}.png` in `CAIRN_SITE_SHOTS_DIR`, or `target/site-shots` under the workspace root.

/// Scenes of the website, in order.
const SITE_SCENES: [&str; 9] = ["home", "instances", "new-instance", "accounts", "library", "settings-appearance", "settings-download", "extensions", "crash"];

#[test]
#[ignore = "writes the website screenshots"]
fn export_site_zh_dark() {
    export_site(Language::ZhCn, false);
}

#[test]
#[ignore = "writes the website screenshots"]
fn export_site_zh_light() {
    export_site(Language::ZhCn, true);
}

#[test]
#[ignore = "writes the website screenshots"]
fn export_site_en_dark() {
    export_site(Language::EnUs, false);
}

#[test]
#[ignore = "writes the website screenshots"]
fn export_site_en_light() {
    export_site(Language::EnUs, true);
}

#[test]
#[ignore = "writes the update panel screenshots"]
fn export_update_panel_screenshots() {
    let out_dir = PathBuf::from(r"C:\Users\34166\.gemini\antigravity\brain\c1bef1bc-f9e9-41ac-9278-7aad1ba1ee44");
    std::fs::create_dir_all(&out_dir).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    cairn_i18n::with_language(Language::ZhCn, || {
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut text = Some(text);
        for (name, light, downloading) in [
            ("update_panel_dark.png", false, false),
            ("update_panel_light.png", true, false),
            ("update_panel_downloading.png", false, true),
        ] {
            let sink: Sink = Arc::new(|_| true);
            let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
            site_model(&mut g.m, Language::ZhCn, light);
            g.ui.sheet = Some(Sheet::Update);
            let info = UpdateInfo {
                current_version: "0.1.0".into(),
                target_version: "0.1.1".into(),
                changelog: "CairnLauncher 0.1.1 更新内容：\n- 新增专属软件更新提示面板与一键原生热替换\n- 优化下载源自动测速算法与资源命中率\n- 修复离线皮肤在特定高版本下的加载问题\n- 提升冷启动性能至 90ms，内存优化 ~15MB".into(),
                download_url: "https://github.com/RustCraftMC/cairn-launcher/releases/download/v0.1.1/CairnLauncher.exe".into(),
                mirror_urls: vec![],
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
                size: 6512340,
            };
            g.m.update_info = Some(info.clone());
            if downloading {
                g.m.update_status = UpdateStatus::Downloading {
                    progress: 0.68,
                    downloaded: 4428391,
                    total: 6512340,
                };
            } else {
                g.m.update_status = UpdateStatus::Available(info);
            }
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let mut app = Headless::with_theme(g, Gui::view, 1120.0, 700.0, 1.5, theme, text.take().unwrap());
            settle_site(&mut app);
            let png_bytes = app.png();
            std::fs::write(out_dir.join(name), png_bytes).unwrap();
            text = Some(app.into_text_system());
        }
    });
}

#[test]
#[ignore = "writes the update panel screenshots"]
fn export_update_panel_screenshots() {
    let out_dir = PathBuf::from(r"C:\Users\34166\.gemini\antigravity\brain\c1bef1bc-f9e9-41ac-9278-7aad1ba1ee44");
    std::fs::create_dir_all(&out_dir).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    cairn_i18n::with_language(Language::ZhCn, || {
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut text = Some(text);
        for (name, light, downloading) in [
            ("update_panel_dark.png", false, false),
            ("update_panel_light.png", true, false),
            ("update_panel_downloading.png", false, true),
        ] {
            let sink: Sink = Arc::new(|_| true);
            let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
            site_model(&mut g.m, Language::ZhCn, light);
            g.ui.sheet = Some(Sheet::Update);
            let info = UpdateInfo {
                current_version: "0.1.0".into(),
                target_version: "0.1.1".into(),
                changelog: "CairnLauncher 0.1.1 更新内容：\n- 新增专属软件更新提示面板与一键原生热替换\n- 优化下载源自动测速算法与资源命中率\n- 修复离线皮肤在特定高版本下的加载问题\n- 提升冷启动性能至 90ms，内存优化 ~15MB".into(),
                download_url: "https://github.com/RustCraftMC/cairn-launcher/releases/download/v0.1.1/CairnLauncher.exe".into(),
                mirror_urls: vec![],
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
                size: 6512340,
            };
            g.m.update_info = Some(info.clone());
            if downloading {
                g.m.update_status = UpdateStatus::Downloading {
                    progress: 0.68,
                    downloaded: 4428391,
                    total: 6512340,
                };
            } else {
                g.m.update_status = UpdateStatus::Available(info);
            }
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let mut app = Headless::with_theme(g, Gui::view, 1120.0, 700.0, 1.5, theme, text.take().unwrap());
            settle_site(&mut app);
            let png_bytes = app.png();
            std::fs::write(out_dir.join(name), png_bytes).unwrap();
            text = Some(app.into_text_system());
        }
    });
}

fn site_shots_dir() -> PathBuf {
    match std::env::var_os("CAIRN_SITE_SHOTS_DIR") {
        Some(dir) => PathBuf::from(dir),
        // This crate is in `crates/cairn-app`, so the workspace root is two levels up.
        None => Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("workspace root").join("target").join("site-shots"),
    }
}

fn export_site(language: Language, light: bool) {
    let dir = site_shots_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let lang = if language == Language::ZhCn { "zh" } else { "en" };
    let mode = if light { "light" } else { "dark" };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    cairn_i18n::with_language(language, || {
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        for scene in SITE_SCENES {
            let sink: Sink = Arc::new(|_| true);
            let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
            site_model(&mut g.m, language, light);
            site_scene(scene, &mut g);
            let theme = crate::style::theme(&g.m.appearance, None, false);
            let mut app = Headless::with_theme(g, Gui::view, 1120.0, 700.0, 1.5, theme, text.take().unwrap());
            settle_site(&mut app);
            std::fs::write(dir.join(format!("{scene}.{lang}.{mode}.png")), app.png()).unwrap();
            text = Some(app.into_text_system());
        }
    });
}

/// Runs the animations to the end, so each picture shows its panel in the settled state.
fn settle_site(app: &mut Headless<Gui>) {
    app.render();
    app.run_animations(240);
    app.advance(Duration::from_millis(400));
    app.render();
    app.run_animations(240);
    app.render();
}

/// Fictional profile: three accounts, six instances, a year of play history and a handful of mods.
fn site_model(m: &mut Model, language: Language, light: bool) {
    m.status = Status::Ready;
    m.config.setup_completed = true;
    m.config.eula_accepted = true;
    m.config.download.use_mcim = true;
    m.config.download.follow_system_proxy = true;
    // The instance list only shows instances of registered game folders. The path is relative on purpose.
    m.config.game_roots = vec![GameRootEntry { name: FOLDER_CURRENT.into(), path: PathBuf::from("games").join(".minecraft") }];
    m.appearance.mode = if light { ThemeMode::Light } else { ThemeMode::Dark };
    m.appearance.language = language;
    let zh = language == Language::ZhCn;

    let li = |kind, v: &str| LoaderInfo { kind, version: v.into() };
    let list = vec![
        site_instance("1.20.1-Fabric", "1.20.1-Fabric", "1.20.1", vec![li(LoaderKind::Fabric, "0.16.14")], true),
        site_instance("1.20.1", "1.20.1", "1.20.1", Vec::new(), false),
        site_instance("Fernwood Survival", "Fernwood Survival", "1.20.1", vec![li(LoaderKind::Forge, "47.3.0")], true),
        site_instance("1.21.4", "1.21.4", "1.21.4", Vec::new(), false),
        site_instance("Quiet Hollows", "Quiet Hollows", "1.21.1", vec![li(LoaderKind::NeoForge, "21.1.77")], true),
        site_instance("Starlit Creative", "Starlit Creative", "1.20.4", vec![li(LoaderKind::Quilt, "0.26.4")], false),
    ];
    m.selected = Some(list[0].id.clone());
    m.instances = Remote::Ready(list);
    m.accounts = Remote::Ready(vec![
        site_account("steve", "Steve", AccountKind::Offline, true),
        site_account("alex", "Alex_2026", AccountKind::Microsoft, false),
        site_account("pixel", "Pixel_Ranger", AccountKind::Yggdrasil, false),
    ]);
    let entry = |id: &str, date: &str| ManifestEntry {
        id: id.into(),
        kind: VersionType::Release,
        url: String::new(),
        time: String::new(),
        release_time: format!("{date}T00:00:00+00:00"),
        sha1: None,
    };
    let versions = [
        ("26.3", "2026-10-28"),
        ("26.2", "2026-09-16"),
        ("26.1", "2026-06-17"),
        ("1.21.4", "2024-12-03"),
        ("1.21.1", "2024-08-08"),
        ("1.20.6", "2024-04-29"),
        ("1.20.4", "2023-12-07"),
        ("1.20.1", "2023-06-07"),
        ("1.19.4", "2023-03-14"),
        ("1.18.2", "2022-02-28"),
        ("1.16.5", "2021-01-14"),
        ("1.12.2", "2017-09-18"),
    ];
    m.manifest = Remote::Ready(versions.iter().map(|(id, date)| entry(id, date)).collect());
    m.latest_release = Some("26.3".into());
    let quote = if zh { "即使身处矿坑深处，也不要忘记仰望星空。" } else { "Even underground, keep looking up." };
    m.hitokoto = Some(Hitokoto { text: quote.into(), from: None });
    m.playtime = site_playtime(cairn_bridge::local_now());
}

fn site_account(id: &str, name: &str, kind: AccountKind, selected: bool) -> AccountInfo {
    AccountInfo { id: id.into(), kind, name: name.into(), uuid: String::new(), skin_url: None, selected, needs_refresh: false, yggdrasil_server: None }
}

/// A fictional instance. Its folder is the built-in "current folder" and its game folder is relative,
/// so no real path can appear on screen.
fn site_instance(id: &str, name: &str, mc: &str, loaders: Vec<LoaderInfo>, isolated: bool) -> Instance {
    let root = PathBuf::from("games").join(".minecraft");
    Instance {
        id: format!("{FOLDER_CURRENT}/{id}"),
        name: name.into(),
        layout: Layout::Standard,
        root: root.clone(),
        version_id: id.into(),
        game_dir: if isolated { root.join("versions").join(id) } else { root },
        settings: InstanceSettings::default(),
        mc_version: Some(mc.into()),
        loaders,
        release_time: None,
        problem: None,
    }
}

/// A year of play history ending today. Busier at weekends, and the last week is the busiest.
fn site_playtime(today: LocalTime) -> PlaytimeStore {
    let mut store = PlaytimeStore::default();
    let base = days_from_civil(i64::from(today.year), i64::from(today.month), i64::from(today.day));
    let mut seed: u64 = 0x2026_1106;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for back in 1..366 {
        // Today stays empty: the home page then shows "last played yesterday" and hides the
        // "played N hours today" line, which does not fill in its number yet.
        let day = base - back;
        // 1970-01-01 was a Thursday, so Saturday and Sunday are where (day + 3) mod 7 is 5 or 6.
        let weekend = matches!((day + 3).rem_euclid(7), 5 | 6);
        let chance = if back < 7 {
            0.9
        } else if weekend {
            0.7
        } else {
            0.4
        };
        if random() > chance {
            continue;
        }
        let hours = if weekend { 1.5 + 2.5 * random() } else { 0.5 + 2.0 * random() };
        let secs = (hours * 3600.0) as u64;
        let pick = random();
        let instance = if pick < 0.6 {
            "1.20.1-Fabric"
        } else if pick < 0.85 {
            "Fernwood Survival"
        } else {
            "Quiet Hollows"
        };
        let (y, m, d) = civil_from_days(day);
        let record = store.records.entry(format!("{y:04}-{m:02}-{d:02}")).or_default();
        record.total_secs += secs;
        *record.instances.entry(format!("{FOLDER_CURRENT}/{instance}")).or_default() += secs;
    }
    store
}

/// Days from 1970-01-01 to a date in the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date for a day count from 1970-01-01. The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Puts the window in the state of one scene: which sheet is open, and what it shows.
fn site_scene(scene: &str, g: &mut Gui) {
    let zh = g.m.appearance.language == Language::ZhCn;
    match scene {
        "home" => {
            g.ui.sheet = None;
            g.m.board = Some(BoardLayout { version: 1, items: site_board(), hide_empty_hint: true });
        }
        "instances" => g.ui.sheet = Some(Sheet::Instances),
        "new-instance" => g.ui.sheet = Some(Sheet::NewInstance),
        "accounts" => g.ui.sheet = Some(Sheet::Accounts),
        "library" => site_library(g),
        "settings-appearance" => site_settings(g, SettingsTab::Appearance),
        "settings-download" => site_settings(g, SettingsTab::Download),
        "extensions" => {
            site_plugins(g, zh);
            site_settings(g, SettingsTab::Extensions);
        }
        "crash" => site_crash(g),
        other => panic!("unknown site scene {other}"),
    }
}

/// Home layout: seven widgets that fill the 12 x 6 grid with no gaps.
fn site_board() -> Vec<BoardItem> {
    let item = |id: &str, kind: &str, x: u16, y: u16, w: u16, h: u16| BoardItem { id: id.into(), kind: kind.into(), x, y, w, h, options: BTreeMap::new() };
    vec![
        item("greeting-1", "greeting", 0, 0, 4, 2),
        item("stats-1", "stats", 4, 0, 4, 2),
        item("activity-1", "activity", 8, 0, 4, 2),
        item("launch-1", "launch", 0, 2, 4, 4),
        item("heatmap-1", "heatmap", 4, 2, 8, 3),
        item("shortcuts-1", "shortcuts", 4, 5, 4, 1),
        item("quote-1", "quote", 8, 5, 4, 1),
    ]
}

fn site_settings(g: &mut Gui, tab: SettingsTab) {
    g.ui.sheet = Some(Sheet::Settings);
    g.ui.settings_tab = Paged::new(tab);
}

/// Mods tab with a Modrinth search for "fabric": installed mods on top, then the online results.
fn site_library(g: &mut Gui) {
    g.ui.sheet = Some(Sheet::Library);
    g.ui.tab = Paged::new(ContentKind::Mods);
    g.ui.search = "fabric".into();
    g.m.online.query = "fabric".into();
    g.m.content.instance = g.m.selected.clone();
    let local = |title: &str, zh_name: &str, file: &str, version: &str, kib: u64| ContentItem {
        path: PathBuf::from(file),
        file_name: file.into(),
        title: title.into(),
        version: Some(version.into()),
        description: None,
        enabled: true,
        is_dir: false,
        size: kib * 1024,
        modified: 1_762_000_000,
        icon_file: None,
        zh_name: Some(zh_name.into()),
        wiki_url: None,
    };
    g.m.content.items = Remote::Ready(vec![
        local("Brass Rails", "黄铜铁轨", "brass-rails-fabric-1.4.2.jar", "1.4.2", 412),
        local("Copper Gears", "紫铜齿轮", "copper-gears-fabric-0.9.0.jar", "0.9.0", 186),
        local("Lantern Core", "灯笼核心", "lantern-core-fabric-2.1.0.jar", "2.1.0", 298),
    ]);
    g.m.content.counts.insert(ContentKind::Mods, 3);
    let online = |slug: &str, title: &str, zh_name: &str, summary: &str, zh_summary: &str, author: &str, downloads: u64, state: OnlineState| OnlineItem {
        source: OnlineSource::Modrinth,
        project_id: slug.into(),
        title: title.into(),
        description: summary.into(),
        author: author.into(),
        downloads,
        icon_url: None,
        state,
        zh_name: Some(zh_name.into()),
        zh_summary: Some(zh_summary.into()),
        wiki_url: None,
        project_url: None,
    };
    g.m.online.results = Remote::Ready(vec![
        online(
            "starlit-lanterns",
            "Starlit Lanterns",
            "星光灯笼",
            "Soft lantern light for caves and night builds, with 40 new fixtures.",
            "为洞穴与夜景补充柔和灯光，新增 40 种灯具。",
            "Steve",
            1_284_300,
            OnlineState::Available,
        ),
        online(
            "quiet-hollows",
            "Quiet Hollows",
            "静默洞穴",
            "Larger caves with hidden ruins, echoing halls and quiet waterfalls.",
            "更宽敞的洞穴，带有隐藏遗迹、回响大厅与安静的瀑布。",
            "Alex_2026",
            642_118,
            OnlineState::Available,
        ),
        online(
            "fernwood-biomes",
            "Fernwood Biomes",
            "蕨木群系",
            "Four forest biomes with fern groves, mossy trees and small cabins.",
            "四个森林群系，包括蕨丛、苔藓古树与小木屋。",
            "Pine & Quill",
            2_051_774,
            OnlineState::Installing(60),
        ),
        online(
            "tidewater-compass",
            "Tidewater Compass",
            "潮汐罗盘",
            "A compass that points to water, shipwrecks and lost chests.",
            "一个能指向水源、沉船与失落宝箱的罗盘。",
            "Steve",
            318_905,
            OnlineState::Available,
        ),
        online(
            "ember-trails",
            "Ember Trails",
            "余烬小径",
            "Glowing paths that guide you between bases at night.",
            "夜晚在据点之间引路的发光小径。",
            "Alex_2026",
            96_430,
            OnlineState::Available,
        ),
        online(
            "moss-lattice",
            "Moss Lattice",
            "苔藓格栅",
            "Decorative lattice blocks in six moss and stone variants.",
            "六种苔石与苔藓风格的装饰格栅方块。",
            "Pine & Quill",
            47_210,
            OnlineState::Available,
        ),
    ]);
}

const FERNWOOD_EN: &str = r#"{"id":"fernwood.status","name":"Fernwood Status","version":"1.2.0","description":"Server status and player count on the home page.","author":"Alex_2026","api":1,"permissions":["network"],"widgets":[{"id":"status","name":"Server status","summary":"Players online and ping","size":[4,2],"min":[2,1],"max":[6,3],"view":[{"element":"text","text":"{data.online}","size":30,"weight":"bold"},{"element":"text","text":"{data.ping}","size":13,"color":"secondary"}]}],"settings":[{"id":"host","title":"Server address","type":"string","default":"mc.example.net"},{"id":"interval","title":"Refresh every","type":"number","min":30,"max":600,"step":30,"default":60},{"id":"compact","title":"Compact layout","type":"boolean","default":false}]}"#;
const FERNWOOD_ZH: &str = r#"{"id":"fernwood.status","name":"Fernwood Status","version":"1.2.0","description":"在首页显示服务器状态与在线人数。","author":"Alex_2026","api":1,"permissions":["network"],"widgets":[{"id":"status","name":"服务器状态","summary":"在线人数与延迟","size":[4,2],"min":[2,1],"max":[6,3],"view":[{"element":"text","text":"{data.online}","size":30,"weight":"bold"},{"element":"text","text":"延迟 {data.ping}","size":13,"color":"secondary"}]}],"settings":[{"id":"host","title":"服务器地址","type":"string","default":"mc.example.net"},{"id":"interval","title":"刷新间隔","type":"number","min":30,"max":600,"step":30,"default":60},{"id":"compact","title":"紧凑布局","type":"boolean","default":false}]}"#;
const LANTERN_EN: &str = r#"{"id":"lantern.gallery","name":"Lantern Gallery","version":"0.9.3","description":"Browse your screenshots from the library.","author":"Steve","api":1,"permissions":["fs"],"settings":[{"id":"sort","title":"Sort by","type":"choice","default":"newest","options":[{"value":"newest","label":"Newest first"},{"value":"name","label":"Name"}]}]}"#;
const LANTERN_ZH: &str = r#"{"id":"lantern.gallery","name":"Lantern Gallery","version":"0.9.3","description":"在资源库中浏览你的截图。","author":"Steve","api":1,"permissions":["fs"],"settings":[{"id":"sort","title":"排序方式","type":"choice","default":"newest","options":[{"value":"newest","label":"最新优先"},{"value":"name","label":"按名称"}]}]}"#;

/// Plugins tab: two declarative plugins, both enabled. The first one is open on its overview.
fn site_plugins(g: &mut Gui, zh: bool) {
    let (fernwood_json, lantern_json) = if zh { (FERNWOOD_ZH, LANTERN_ZH) } else { (FERNWOOD_EN, LANTERN_EN) };
    let fernwood = Manifest::parse(fernwood_json.as_bytes()).expect("fixture manifest");
    let lantern = Manifest::parse(lantern_json.as_bytes()).expect("fixture manifest");
    g.m.plugins.state.enable(&fernwood);
    g.m.plugins.state.enable(&lantern);
    g.m.plugins.entries = vec![
        PluginEntry { dir: PathBuf::from("plugins").join(&fernwood.id), manifest: Some(fernwood.clone()), issue: None },
        PluginEntry { dir: PathBuf::from("plugins").join(&lantern.id), manifest: Some(lantern), issue: None },
    ];
    let data = BTreeMap::from([("online".to_string(), "12 / 40".to_string()), ("ping".to_string(), "38 ms".to_string())]);
    g.m.plugins.live.insert(widget_kind("fernwood.status", "status"), LiveWidget { data, view: None, view_gen: 0 });
    g.ui.extensions.selected = Some("plugin:fernwood.status".into());
}

/// A game crash with two findings and the key log lines open.
fn site_crash(g: &mut Gui) {
    g.m.crashes = vec![CrashInfo {
        id: 7,
        kind: CrashKind::Game { game: 7, code: Some(1), run_secs: 812 },
        instance: format!("{FOLDER_CURRENT}/1.20.1-Fabric"),
        analysis: CrashAnalysis {
            findings: vec![
                Finding { cause: Cause::OutOfMemory, details: Vec::new() },
                Finding { cause: Cause::ModMissingDependency, details: vec!["Ember Trails".into(), "Moss Lattice".into()] },
            ],
            key_lines: vec![
                "java.lang.OutOfMemoryError: Java heap space".into(),
                "at net.minecraft.client.renderer.LevelRenderer.renderLevel(LevelRenderer.java:1231)".into(),
                "at net.fabricmc.loader.impl.launch.knot.KnotClassLoader.loadClass(KnotClassLoader.java:94)".into(),
            ],
        },
    }];
    g.ui.sheet = Some(Sheet::Crash(7));
    g.ui.crash_lines_open = true;
}
