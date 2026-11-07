//! Settings: category tabs (Appearance, Game, Java, Downloads, Plugins, About) above the category content.

use cairn_i18n::tr;

use cairn_bridge::{AccentChoice, Appearance, GcPolicy, IsolationPolicy, OfflineSkinSource, Remote, SourceStrategy, ThemeMode};
use cairn_ui::{Align, El, Icon, Insets, Rect, Styled, button, checkbox, column, label, list_item, row, scroll, tab, tag, text_input};

use super::controls::{dropdown_button, java_list, seg_track, segmented, setting_row};
use super::memory;
use super::version::window_editor;
use super::{frame, header, loading_row, page_switch, section, swap};
use crate::gui::{DropdownKind, Gui, SettingsTab, Sheet};
use crate::style::{GREEN, RED, accent_color, fg};
use crate::widgets::{logo_mark, swatch};

fn tabs_def() -> [(SettingsTab, &'static str, Icon); 6] {
    [
        (SettingsTab::Appearance, tr!("settings.appearance"), Icon::Image),
        (SettingsTab::Game, tr!("settings.game"), Icon::Gamepad),
        (SettingsTab::Java, tr!("settings.java"), Icon::Cup),
        (SettingsTab::Download, tr!("settings.download"), Icon::Download),
        (SettingsTab::Extensions, tr!("settings.extensions"), Icon::Puzzle),
        (SettingsTab::About, tr!("settings.about"), Icon::Info),
    ]
}

/// A category page made of setting rows, in a scroll area of its own.
fn rows_page(rows: Vec<El<Gui>>) -> El<Gui> {
    scroll().grow(1.0).min_height(0.0).align(Align::Stretch).padding(Insets::new(10.0, 16.0, 20.0, 16.0)).gap(6.0).children(rows).into()
}

pub fn view(g: &Gui) -> El<Gui> {
    let current = g.ui.settings_tab.shown;
    let target = g.ui.settings_tab.target();
    let tabs =
        row().gap(2.0).children(tabs_def().map(|(t, name, ic)| tab(name).icon(ic).selected(t == target).on_click(move |g: &mut Gui| g.ui.settings_tab.go(t))));
    let page = match current {
        SettingsTab::Appearance => rows_page(appearance(g)),
        SettingsTab::Game => rows_page(game(g)),
        SettingsTab::Java => rows_page(java(g)),
        SettingsTab::Download => rows_page(sources(g)),
        // The manager has a list and a detail pane that scroll on their own, so it is not a page of rows.
        SettingsTab::Extensions => super::extensions::view(g),
        SettingsTab::About => rows_page(about_page(g)),
    };
    let body = page_switch(
        ("settings_tab", current as u8),
        g.ui.settings_tab.leaving(),
        |g: &mut Gui| {
            g.ui.settings_tab.arrive();
        },
        page,
    );
    frame(header(tabs, Vec::new()), body, None)
}

fn set(g: &mut Gui, f: impl FnOnce(&mut Appearance)) {
    let mut ap = g.m.appearance.clone();
    f(&mut ap);
    g.bridge.set_appearance(&mut g.m, ap);
}

fn appearance(g: &Gui) -> Vec<El<Gui>> {
    let ap = &g.m.appearance;
    let wallpaper = g.m.wallpaper.ready();

    let modes =
        [(ThemeMode::Dark, tr!("settings.theme_dark")), (ThemeMode::Light, tr!("settings.theme_light")), (ThemeMode::System, tr!("settings.theme_system"))];
    let mode = seg_track().options(modes.map(|(m, name)| tab(name).selected(ap.mode == m).on_click(move |g: &mut Gui| set(g, |a| a.mode = m))));
    let presets = [AccentChoice::Gold, AccentChoice::Blue, AccentChoice::Green, AccentChoice::Purple, AccentChoice::Rose, AccentChoice::Teal];
    let mut swatches =
        row().gap(4.0).children(presets.map(|c| swatch::<Gui>(accent_color(c, None), ap.accent == c).on_click(move |g: &mut Gui| set(g, |a| a.accent = c))));
    swatches = swatches.child(
        swatch::<Gui>(accent_color(AccentChoice::FromWallpaper, wallpaper), ap.accent == AccentChoice::FromWallpaper)
            .icon(Icon::Image)
            .on_click(|g: &mut Gui| set(g, |a| a.accent = AccentChoice::FromWallpaper)),
    );
    swatches = swatches.child(
        swatch::<Gui>(
            g.ui.accent_picker.as_ref().map(|p| p.color()).unwrap_or_else(|| accent_color(ap.accent, wallpaper)),
            matches!(ap.accent, AccentChoice::Custom(_)),
        )
        .icon(Icon::More)
        .label(tr!("settings.accent_custom"))
        .on_click_rect(super::accent_picker::open),
    );
    let accent_hint = match (ap.accent, wallpaper) {
        (AccentChoice::FromWallpaper, None) => tr!("settings.accent_hint_auto"),
        (AccentChoice::FromWallpaper, Some(_)) => tr!("settings.accent_hint_picked"),
        _ => tr!("settings.accent_hint_default"),
    };

    let (wp_title, wp_detail) = match (&ap.wallpaper, &g.m.wallpaper) {
        (None, _) => (tr!("settings.wallpaper_default").to_string(), tr!("settings.wallpaper_custom_desc").to_string()),
        (Some(p), Remote::Failed(e)) => (file_name(p), e.clone()),
        (Some(p), Remote::Ready(w)) => (file_name(p), format!("{} × {}", w.width, w.height)),
        (Some(p), _) => (file_name(p), tr!("settings.wallpaper_reading").to_string()),
    };
    let wp_buttons = row().gap(8.0).child(button(tr!("settings.wallpaper_pick")).icon(Icon::Folder).on_click(|g: &mut Gui| g.bridge.pick_wallpaper(&mut g.m)));
    let wp_buttons =
        wp_buttons.child(button(tr!("settings.wallpaper_reset")).enabled(ap.wallpaper.is_some()).on_click(|g: &mut Gui| set(g, |a| a.wallpaper = None)));

    let dims = [
        (0.0, tr!("settings.dim_none")),
        (0.2, tr!("settings.dim_low")),
        (0.35, tr!("settings.dim_medium")),
        (0.55, tr!("settings.dim_deep")),
        (0.75, tr!("settings.dim_max")),
    ];
    let dim = seg_track()
        .options(dims.map(|(v, name)| tab(name).selected((ap.wallpaper_dim - v).abs() < 0.05).on_click(move |g: &mut Gui| set(g, |a| a.wallpaper_dim = v))));
    let vignettes = [(0.0, tr!("settings.vignette_none")), (0.35, tr!("settings.vignette_slight")), (0.65, tr!("settings.vignette_shadow"))];
    let vignette = seg_track().options(
        vignettes
            .map(|(v, name)| tab(name).selected((ap.wallpaper_vignette - v).abs() < 0.05).on_click(move |g: &mut Gui| set(g, |a| a.wallpaper_vignette = v))),
    );
    let wp_blurs =
        [(0.0, tr!("settings.blur_none")), (12.0, tr!("settings.blur_low")), (24.0, tr!("settings.blur_medium")), (44.0, tr!("settings.blur_bitmap"))];
    let wp_blur = seg_track().options(
        wp_blurs.map(|(v, name)| tab(name).selected((ap.wallpaper_blur - v).abs() < 2.0).on_click(move |g: &mut Gui| set(g, |a| a.wallpaper_blur = v))),
    );
    let blurs = [
        (0.0, tr!("settings.acrylic_none")),
        (18.0, tr!("settings.acrylic_light")),
        (34.0, tr!("settings.acrylic_medium")),
        (56.0, tr!("settings.acrylic_strong")),
    ];
    let blur = seg_track().options(blurs.map(|(v, name)| tab(name).selected((ap.blur - v).abs() < 0.5).on_click(move |g: &mut Gui| set(g, |a| a.blur = v))));

    let cur_lang_label = match ap.language {
        cairn_bridge::Language::Auto => tr!("settings.lang_auto"),
        cairn_bridge::Language::ZhCn => tr!("settings.lang_zh_cn"),
        cairn_bridge::Language::EnUs => tr!("settings.lang_en_us"),
        cairn_bridge::Language::JaJp => tr!("settings.lang_ja_jp"),
    };
    let lang_ctrl = dropdown_button(cur_lang_label, |g: &mut Gui, rect: Rect| {
        g.open_dropdown(DropdownKind::Language, rect);
    })
    .icon(Icon::Globe)
    .min_width(180.0);

    let mut out = vec![
        section(tr!("settings.theme_and_language"), Icon::Sun),
        setting_row(tr!("settings.language"), tr!("settings.language_desc"), lang_ctrl),
        setting_row(tr!("settings.theme_mode"), tr!("settings.theme_desc"), mode),
        setting_row(tr!("settings.accent_color"), accent_hint, swatches),
        section(tr!("settings.wallpaper"), Icon::Image),
        setting_row(&wp_title, &wp_detail, wp_buttons),
    ];
    if ap.wallpaper.is_some() {
        out.push(setting_row(tr!("settings.wallpaper_dim"), tr!("settings.wallpaper_dim_desc"), dim));
        out.push(setting_row(tr!("settings.vignette_shadow"), tr!("settings.vignette_shadow_desc"), vignette));
        out.push(setting_row(tr!("settings.wallpaper_blur"), tr!("settings.wallpaper_blur_desc"), wp_blur));
    }
    out.push(setting_row(tr!("settings.window_blur"), tr!("settings.window_blur_desc"), blur));
    let borders = seg_track().options(
        [(true, tr!("settings.borders_on")), (false, tr!("settings.borders_off"))]
            .map(|(on, name)| tab(name).selected(ap.borders == on).on_click(move |g: &mut Gui| set(g, |a| a.borders = on))),
    );
    out.push(setting_row(tr!("settings.borders"), tr!("settings.borders_desc"), borders));

    // The widget layout is edited on the home page. Here: an entry point and a reset.
    let board = row()
        .gap(8.0)
        .child(button(tr!("settings.board_edit")).on_click(|g: &mut Gui| {
            g.close();
            crate::board::enter_edit(g);
        }))
        .child(button(tr!("settings.board_reset_default")).enabled(g.m.board.is_some()).on_click(|g: &mut Gui| {
            g.close();
            crate::board::reset_from_settings(g);
        }));
    out.push(section(tr!("settings.board_widgets"), Icon::Home));
    out.push(setting_row(tr!("settings.board_widgets"), tr!("settings.board_widgets_desc"), board));

    out
}

fn game(g: &Gui) -> Vec<El<Gui>> {
    let launch = &g.m.config.launch;
    let window = window_editor(launch.window.clone(), |g: &mut Gui, f| {
        let next = f(g.m.config.launch.window.clone());
        g.bridge.edit_config(&mut g.m, move |c| c.launch.window = next.clone());
    });
    let name = text_input(&launch.launcher_name).placeholder("CairnLauncher").width(220.0).on_change(|g: &mut Gui, t: String| {
        g.bridge.edit_config(&mut g.m, move |c| c.launch.launcher_name = t.clone());
    });
    let gc = segmented(
        &[(GcPolicy::Auto, tr!("settings.gc_auto")), (GcPolicy::G1, "G1"), (GcPolicy::Zgc, "ZGC"), (GcPolicy::None, tr!("settings.gc_revert"))],
        launch.gc,
        |g: &mut Gui, p| g.bridge.edit_config(&mut g.m, move |c| c.launch.gc = p),
    );
    let skin_enabled =
        segmented(&[(true, tr!("settings.skin_enabled")), (false, tr!("settings.skin_disabled"))], launch.offline_skin.enabled, |g: &mut Gui, p| {
            g.bridge.edit_config(&mut g.m, move |c| c.launch.offline_skin.enabled = p)
        });
    let skin_source = segmented(
        &[
            (OfflineSkinSource::Mojang, tr!("settings.skin_source_mojang")),
            (OfflineSkinSource::LittleSkin, tr!("settings.skin_source_littleskin")),
            (OfflineSkinSource::Custom, tr!("settings.skin_source_custom")),
        ],
        launch.offline_skin.source,
        |g: &mut Gui, p| g.bridge.edit_config(&mut g.m, move |c| c.launch.offline_skin.source = p),
    );
    let custom_skin = text_input(launch.offline_skin.custom_skin.as_deref().unwrap_or(""))
        .placeholder(tr!("settings.skin_custom_placeholder"))
        .width(320.0)
        .on_change(|g: &mut Gui, t: String| {
            let t = if t.trim().is_empty() { None } else { Some(t) };
            g.bridge.edit_config(&mut g.m, move |c| c.launch.offline_skin.custom_skin = t.clone());
        });
    let jvm = text_input(&launch.extra_jvm_args).placeholder(tr!("settings.jvm_args_placeholder")).width(320.0).on_change(|g: &mut Gui, t: String| {
        g.bridge.edit_config(&mut g.m, move |c| c.launch.extra_jvm_args = t.clone());
    });
    let import_btn = button(tr!("settings.import_external_btn"))
        .icon(Icon::Download)
        .on_click(|g: &mut Gui| g.open(Sheet::ImportVersions));
    let args = text_input(&launch.extra_game_args).placeholder(tr!("settings.game_args_placeholder")).width(320.0).on_change(|g: &mut Gui, t: String| {
        g.bridge.edit_config(&mut g.m, move |c| c.launch.extra_game_args = t.clone());
    });
    let mut rows = vec![
        section(tr!("settings.version_migration"), Icon::Download),
        setting_row(tr!("settings.import_external"), tr!("settings.import_external_desc"), import_btn),
        label(tr!("settings.global_settings_notice")).caption().margin(Insets::new(6.0, 0.0, 6.0, 8.0)).into(),
        section(tr!("settings.version_isolation"), Icon::Folder),
        isolation_policy(g),
        section(tr!("settings.launch_preparation"), Icon::Download),
        setting_row(
            tr!("settings.auto_complete_files"),
            tr!("settings.auto_complete_files_desc"),
            checkbox(launch.auto_complete_files).on_change(|g: &mut Gui, on| g.bridge.edit_config(&mut g.m, move |c| c.launch.auto_complete_files = on)),
        ),
        section(tr!("settings.window_and_info"), Icon::Gamepad),
        setting_row(tr!("settings.game_window"), tr!("settings.game_window_desc"), window),
        setting_row(tr!("settings.custom_info"), tr!("settings.custom_info_desc"), name),
        section(tr!("settings.skin_title"), Icon::User),
        setting_row(tr!("settings.skin_title"), tr!("settings.skin_enabled_desc"), skin_enabled),
        setting_row(tr!("settings.skin_source"), tr!("settings.skin_source_desc"), skin_source),
    ];
    if launch.offline_skin.source == OfflineSkinSource::Custom {
        rows.push(setting_row(tr!("settings.skin_custom_url"), tr!("settings.skin_custom_url_desc"), custom_skin));
    }
    rows.extend([
        section(tr!("settings.memory_allocation"), Icon::Chip),
        column()
            .fill_width()
            .padding(Insets::new(2.0, 8.0, 6.0, 8.0))
            .child(memory::block(g, launch.memory, None, |g: &mut Gui, p| g.bridge.edit_config(&mut g.m, move |c| c.launch.memory = p)))
            .into(),
        section(tr!("common.edit"), Icon::Gear),
        setting_row(tr!("settings.gc_auto"), tr!("settings.gc_desc"), gc),
        setting_row(tr!("settings.jvm_args"), tr!("settings.jvm_args_desc"), jvm),
        setting_row(tr!("settings.game_args"), tr!("settings.game_args_desc"), args),
    ]);
    rows
}

fn java(g: &Gui) -> Vec<El<Gui>> {
    let preferred = g.m.config.java.preferred.clone();
    let mut out: Vec<El<Gui>> = vec![
        section(tr!("settings.java_in_use"), Icon::Cup),
        list_item()
            .selected(preferred.is_none())
            .on_click(|g: &mut Gui| g.bridge.edit_config(&mut g.m, |c| c.java.preferred = None))
            .child(
                column()
                    .grow(1.0)
                    .gap(2.0)
                    .child(label(tr!("settings.java_auto_title")).weight(700).color(fg(0.92)))
                    .child(label(tr!("settings.java_auto_desc")).caption()),
            )
            .into(),
    ];
    out.extend(java_list(g, preferred.as_deref(), |g: &mut Gui, p| g.bridge.edit_config(&mut g.m, move |c| c.java.preferred = Some(p.clone()))));

    out.push(
        row()
            .fill_width()
            .gap(10.0)
            .padding(Insets::new(6.0, 6.0, 10.0, 6.0))
            .child(
                checkbox(g.m.config.java.auto_download)
                    .label(tr!("settings.java_auto_download"))
                    .grow(1.0)
                    .on_change(|g: &mut Gui, on| g.bridge.edit_config(&mut g.m, move |c| c.java.auto_download = on)),
            )
            .child(button(tr!("settings.java_add")).icon(Icon::Plus).on_click(|g: &mut Gui| g.bridge.pick_java(&mut g.m)))
            .child(button(tr!("settings.java_rescan")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.load_javas(&mut g.m, true)))
            .into(),
    );
    out
}

fn sources(g: &Gui) -> Vec<El<Gui>> {
    let current = g.m.config.download.strategy;
    let options = [
        (SourceStrategy::Auto, tr!("settings.source_auto")),
        (SourceStrategy::PreferOfficial, tr!("settings.source_prefer_official")),
        (SourceStrategy::PreferMirror, tr!("settings.source_prefer_mirror")),
        (SourceStrategy::OfficialOnly, tr!("settings.source_official_only")),
        (SourceStrategy::MirrorOnly, tr!("settings.source_mirror_only")),
    ];
    let mut out: Vec<El<Gui>> = vec![
        section(tr!("settings.download_sources"), Icon::Download),
        row()
            .fill_width()
            .gap(2.0)
            .padding(Insets::new(4.0, 2.0, 4.0, 2.0))
            .children(options.map(|(s, name)| {
                tab(name).selected(current == s).on_click(move |g: &mut Gui| g.bridge.edit_config(&mut g.m, move |c| c.download.strategy = s))
            }))
            .child(row().grow(1.0))
            .child(button(tr!("settings.source_test_speed")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.test_sources(&mut g.m)))
            .into(),
        label(tr!("settings.source_auto_desc")).caption().margin(Insets::symmetric(6.0, 2.0)).into(),
    ];
    out.push(section(tr!("settings.proxy_section"), Icon::Globe));
    out.push(setting_row(
        tr!("settings.proxy_follow_sys"),
        tr!("settings.proxy_follow_sys_desc"),
        checkbox(g.m.config.download.follow_system_proxy).on_change(|g: &mut Gui, on| {
            g.bridge.edit_config(&mut g.m, move |c| c.download.follow_system_proxy = on);
        }),
    ));
    out.push(section(tr!("settings.platform_accel_section"), Icon::Package));
    out.push(setting_row(
        tr!("settings.mcim_title"),
        tr!("settings.mcim_desc"),
        checkbox(g.m.config.download.use_mcim).on_change(|g: &mut Gui, on| g.bridge.edit_config(&mut g.m, move |c| c.download.use_mcim = on)),
    ));
    let key = g.m.config.credentials.curseforge_api_key.clone().unwrap_or_default();
    out.push(setting_row(
        tr!("settings.cf_key_title"),
        tr!("settings.cf_key_desc"),
        text_input(&key).placeholder(tr!("settings.cf_key_placeholder")).width(260.0).on_change(|g: &mut Gui, t: String| {
            g.bridge.edit_config(&mut g.m, move |c| c.credentials.curseforge_api_key = (!t.trim().is_empty()).then(|| t.trim().to_string()));
        }),
    ));
    out.push(section(tr!("settings.source_speed_section"), Icon::Refresh));
    match &g.m.sources {
        Remote::Ready(list) => out.extend(list.iter().map(|s| {
            let (text, color) = match s.latency_ms {
                Some(ms) => (format!("{ms} ms"), if ms < 300 { GREEN } else { fg(0.7) }),
                None => (tr!("settings.source_offline").to_string(), RED),
            };
            row()
                .fill_width()
                .padding(Insets::symmetric(12.0, 5.0))
                .child(label(&s.name).grow(1.0))
                .child(label(text).font_size(13.0).weight(700).color(color))
                .into()
        })),
        Remote::Loading => out.push(loading_row(tr!("settings.source_testing"))),
        Remote::Failed(e) => out.push(label(e.as_str()).caption().color(RED).into()),
        Remote::Idle => {}
    }
    out
}

fn about_page(g: &Gui) -> Vec<El<Gui>> {
    let dir = g.m.data_dir.clone();
    let text = dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| tr!("settings.data_dir_unspecified").to_string());

    let hero = column()
        .fill_width()
        .align(Align::Center)
        .padding(Insets::new(16.0, 16.0, 18.0, 16.0))
        .gap(8.0)
        .child(logo_mark(92.0))
        .child(label(tr!("settings.about_slogan")).font_size(13.0).color(fg(0.68)))
        .child(row().gap(6.0).child(tag(format!("v{}", env!("CARGO_PKG_VERSION"))).accent()).child(tag("Windows x86_64")).child(tag("Pure Rust")));

    let (update_title, update_desc, update_action) = match &g.m.update_status {
        cairn_bridge::UpdateStatus::Idle => (
            tr!("settings.check_update").to_string(),
            tr!("settings.auto_check_desc").to_string(),
            button(tr!("settings.check_update")).icon(Icon::Refresh).on_click(|g: &mut Gui| {
                g.bridge.check_update(&mut g.m, false);
            }),
        ),
        cairn_bridge::UpdateStatus::Checking => (
            tr!("settings.checking_update").to_string(),
            tr!("settings.auto_check_desc").to_string(),
            button(tr!("settings.checking_update")).icon(Icon::Refresh),
        ),
        cairn_bridge::UpdateStatus::UpToDate => (
            tr!("settings.up_to_date").to_string(),
            tr!("settings.auto_check_desc").to_string(),
            button(tr!("settings.check_update")).icon(Icon::Refresh).on_click(|g: &mut Gui| {
                g.bridge.check_update(&mut g.m, false);
            }),
        ),
        cairn_bridge::UpdateStatus::Available(info) => (
            tr!("settings.update_available").replace("{version}", &info.target_version),
            info.changelog.clone(),
            button(tr!("update.view_details")).icon(Icon::Download).on_click(|g: &mut Gui| {
                g.open(crate::gui::Sheet::Update);
            }),
        ),
        cairn_bridge::UpdateStatus::Downloading { progress, downloaded, total } => {
            let percent = (progress * 100.0) as u32;
            let progress_text = format!("{:.1} / {:.1} MiB", *downloaded as f64 / 1048576.0, *total as f64 / 1048576.0);
            (
                tr!("settings.downloading_update").replace("{percent}", &percent.to_string()),
                progress_text,
                button(format!("{percent}%")).icon(Icon::Download),
            )
        }
        cairn_bridge::UpdateStatus::ReadyToRestart => (
            tr!("settings.ready_to_restart").to_string(),
            tr!("settings.auto_check_desc").to_string(),
            button(tr!("settings.restart_btn")).icon(Icon::Refresh).on_click(|g: &mut Gui| {
                g.bridge.apply_launcher_update(&mut g.m);
            }),
        ),
        cairn_bridge::UpdateStatus::Failed(err) => (
            tr!("settings.update_failed").to_string(),
            err.clone(),
            button(tr!("settings.check_update")).icon(Icon::Refresh).on_click(|g: &mut Gui| {
                g.bridge.check_update(&mut g.m, false);
            }),
        ),
    };

    vec![
        hero.into(),
        section(tr!("settings.update_section"), Icon::Refresh),
        setting_row(&update_title, &update_desc, update_action),
        setting_row(
            tr!("settings.auto_check_update"),
            tr!("settings.auto_check_desc"),
            checkbox(g.m.config.update.auto_check).on_change(|g: &mut Gui, on| {
                g.bridge.set_auto_check_update(&mut g.m, on);
            }),
        ),
        section(tr!("settings.about_repo_title"), Icon::Globe),
        setting_row(
            tr!("settings.about_cairn_repo"),
            "https://github.com/RustCraftMC/cairn-launcher",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/RustCraftMC/cairn-launcher");
            }),
        ),
        setting_row(
            tr!("settings.about_ui_repo"),
            "https://github.com/xiaojiang233/cairn-ui",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/xiaojiang233/cairn-ui");
            }),
        ),
        section(tr!("settings.about_developer"), Icon::User),
        setting_row(
            "xiaojiang233",
            "https://github.com/xiaojiang233",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/xiaojiang233");
            }),
        ),
        setting_row(
            "Really1337",
            "https://github.com/feijx3/",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/feijx3/");
            }),
        ),
        section(tr!("settings.about_credits"), Icon::Info),
        setting_row(
            "Plain Craft Launcher 2",
            "https://github.com/Meloong-Git/PCL",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/Meloong-Git/PCL");
            }),
        ),
        setting_row(
            "Hello Minecraft! Launcher",
            "https://github.com/HMCL-dev/HMCL",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/HMCL-dev/HMCL");
            }),
        ),
        setting_row(
            "Prism Launcher",
            "https://github.com/PrismLauncher/PrismLauncher",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/PrismLauncher/PrismLauncher");
            }),
        ),
        setting_row(
            "b-launcher",
            "https://github.com/cubk1/b-launcher",
            button("GitHub").icon(Icon::ExternalLink).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://github.com/cubk1/b-launcher");
            }),
        ),
        setting_row(
            tr!("settings.about_bmclapi"),
            "https://bmclapidoc.bangbang93.com/",
            button(tr!("common.open_browser")).icon(Icon::Globe).on_click(|g: &mut Gui| {
                g.bridge.open_url("https://bmclapidoc.bangbang93.com/");
            }),
        ),
        section(tr!("settings.about_compliance"), Icon::Gear),
        setting_row(
            tr!("settings.about_data_root"),
            &text,
            button(tr!("settings.about_open_dir")).icon(Icon::Folder).enabled(dir.is_some()).on_click(move |g: &mut Gui| {
                if let Some(d) = &dir {
                    g.bridge.open_path(d);
                }
            }),
        ),
        setting_row(
            tr!("settings.about_eula_title"),
            tr!("settings.about_eula_desc"),
            button(tr!("settings.about_view_rules")).icon(Icon::Info).on_click(|g: &mut Gui| {
                g.open(Sheet::Eula);
            }),
        ),
    ]
}

/// Isolation policy for new installs: whether saves and mods go into the version's own
/// `versions/<version>/` folder. Installed versions are not affected by this setting.
/// The five tiers form one segmented control. Only the selected tier's description is shown,
/// since all five would be a large block of text that outweighs the other settings.
fn isolation_policy(g: &Gui) -> El<Gui> {
    let current = g.m.config.launch.isolation;
    let options = [
        (IsolationPolicy::Off, tr!("settings.isolation_off"), tr!("settings.isolation_off_detail")),
        (IsolationPolicy::Mods, tr!("settings.isolation_mods"), tr!("settings.isolation_mods_detail")),
        (IsolationPolicy::Snapshots, tr!("settings.isolation_snapshots"), tr!("settings.isolation_snapshots_detail")),
        (IsolationPolicy::ModsAndSnapshots, tr!("settings.isolation_mods_and_snapshots"), tr!("settings.isolation_mods_and_snapshots_detail")),
        (IsolationPolicy::All, tr!("settings.isolation_all"), tr!("settings.isolation_all_detail")),
    ];
    let names = options.map(|(p, name, _)| (p, name));
    let detail = options.iter().find(|(p, ..)| *p == current).map_or("", |(.., detail)| *detail);
    let control = segmented(&names, current, |g: &mut Gui, p| g.bridge.edit_config(&mut g.m, move |c| c.launch.isolation = p));
    // Two lines are reserved for the description, so the rows below do not jump.
    // The new text fades in when the tier changes.
    let detail = column().fill_width().min_height(36.0).child(swap(("isolation-detail", detail), label(detail).caption().color(fg(0.7)).fill_width()));
    column()
        .fill_width()
        .gap(8.0)
        .align(Align::Stretch)
        .padding(Insets::new(0.0, 8.0, 6.0, 8.0))
        .child(label(tr!("settings.isolation_setting_desc")).caption())
        .child(control)
        .child(detail)
        .into()
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_bridge::{Bridge, Language, Sink};
    use cairn_ui::testing::Headless;
    use std::sync::Arc;

    #[test]
    fn game_file_completion_toggle_changes_only_its_setting_and_keeps_its_place() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            cairn_i18n::with_language(language, || {
                let sink: Sink = Arc::new(|_| true);
                let gui = Gui::new(Bridge::start(rt.handle().clone(), sink));
                let theme = crate::style::theme(&gui.m.appearance, None, false);
                let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
                let mut app =
                    Headless::with_theme(gui, |g: &Gui| column().width(760.0).align(Align::Stretch).children(game(g)), 800.0, 1000.0, 1.0, theme, text);
                let toggle = app.find_role(cairn_ui::Role::Checkbox)[0].bounds;
                let title = app.find(tr!("settings.auto_complete_files")).unwrap().bounds;
                assert!(app.state().m.config.launch.auto_complete_files);
                for enabled in [false, true] {
                    app.click(toggle.center());
                    assert_eq!(app.state().m.config.launch.auto_complete_files, enabled);
                    assert!(app.state().m.config.java.auto_download);
                    assert_eq!(app.find_role(cairn_ui::Role::Checkbox)[0].bounds, toggle);
                    assert_eq!(app.find(tr!("settings.auto_complete_files")).unwrap().bounds, title);
                }
            });
        }
    }

    #[test]
    fn isolation_policy_is_one_row_with_only_the_selected_description() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            cairn_i18n::with_language(language, || {
                let sink: Sink = Arc::new(|_| true);
                let gui = Gui::new(Bridge::start(rt.handle().clone(), sink));
                assert_eq!(gui.m.config.launch.isolation, IsolationPolicy::ModsAndSnapshots, "默认是推荐的那一档");
                let theme = crate::style::theme(&gui.m.appearance, None, false);
                let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
                let mut app =
                    Headless::with_theme(gui, |g: &Gui| column().width(760.0).align(Align::Stretch).child(isolation_policy(g)), 800.0, 400.0, 1.0, theme, text);
                app.render();
                let names = [
                    tr!("settings.isolation_off"),
                    tr!("settings.isolation_mods"),
                    tr!("settings.isolation_snapshots"),
                    tr!("settings.isolation_mods_and_snapshots"),
                    tr!("settings.isolation_all"),
                ];
                let boxes: Vec<_> = names.iter().map(|n| app.find(n).unwrap_or_else(|| panic!("{language:?}: 缺 {n}")).bounds).collect();
                assert!(boxes.iter().all(|b| (b.y - boxes[0].y).abs() < 1.0 && b.right() <= 760.0), "{language:?}: {boxes:?}");
                assert!(app.find(tr!("settings.isolation_mods_and_snapshots_detail")).is_some());
                assert!(app.find(tr!("settings.isolation_off_detail")).is_none() && app.find(tr!("settings.isolation_all_detail")).is_none());
                let top = app.find(tr!("settings.isolation_setting_desc")).unwrap().bounds.y;
                let bottom = app.find(tr!("settings.isolation_mods_and_snapshots_detail")).unwrap().bounds.bottom();
                assert!(bottom - top < 110.0, "{language:?}: 这一块有 {} 高", bottom - top);

                app.click(boxes[0].center());
                assert_eq!(app.state().m.config.launch.isolation, IsolationPolicy::Off);
                app.run_animations(30);
                assert!(app.find(tr!("settings.isolation_off_detail")).is_some());
                assert!(app.find(tr!("settings.isolation_mods_and_snapshots_detail")).is_none());
            });
        }
    }
}
