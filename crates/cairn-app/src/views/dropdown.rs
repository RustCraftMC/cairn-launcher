//! Popup view of the shared dropdown picker.

use cairn_bridge::Language;
use cairn_i18n::tr;
use std::time::Duration;

use cairn_ui::{El, Icon, Styled, anchored, menu, menu_item, reveal};

use crate::gui::{ActiveDropdown, DropdownKind, Gui};

pub fn view(g: &Gui, drop: &ActiveDropdown, leaving: bool) -> El<Gui> {
    let mut anch = anchored(drop.at).anchor_right(drop.align_right);
    if !leaving {
        anch = anch.on_dismiss(|g: &mut Gui| g.close_dropdown());
    }
    anch.child(
        reveal()
            .from_above()
            .offset(8.0)
            .duration(Duration::from_millis(160))
            .exit_duration(Duration::from_millis(120))
            .key(("dropdown_reveal", drop.kind.clone()))
            .leaving(leaving)
            .on_left(|g: &mut Gui| {
                g.ui.dropdown_leaving = None;
                g.apply_dropdown_choice();
            })
            .child(dropdown_content(g, drop)),
    )
    .into()
}

fn dropdown_content(g: &Gui, drop: &ActiveDropdown) -> El<Gui> {
    match &drop.kind {
        DropdownKind::Language => language_menu(g, drop.min_width),
        DropdownKind::AddExtension => super::extensions::add_menu(g, drop.min_width),
        DropdownKind::OnlineLinks { source, platform_url, wiki_url, .. } => {
            online_links_menu(g, *source, platform_url, wiki_url.as_deref(), drop.min_width)
        }
    }
}

fn online_links_menu(_g: &Gui, source: cairn_bridge::OnlineSource, platform_url: &str, wiki_url: Option<&str>, min_width: f32) -> El<Gui> {
    let p_url = platform_url.to_string();
    let platform_label = match source {
        cairn_bridge::OnlineSource::Modrinth => tr!("library.link_modrinth"),
        cairn_bridge::OnlineSource::CurseForge => tr!("library.link_curseforge"),
    };
    let mut m = menu()
        .min_width(min_width.max(150.0))
        .child(
            menu_item(platform_label)
                .icon(Icon::Globe)
                .on_click(move |g: &mut Gui| {
                    g.close_dropdown();
                    g.bridge.open_url(&p_url);
                }),
        );
    if let Some(w_url) = wiki_url {
        let w_url = w_url.to_string();
        m = m.child(
            menu_item(tr!("library.link_mcmod"))
                .icon(Icon::Doc)
                .on_click(move |g: &mut Gui| {
                    g.close_dropdown();
                    g.bridge.open_url(&w_url);
                }),
        );
    }
    m.into()
}

fn language_menu(g: &Gui, min_width: f32) -> El<Gui> {
    let cur = g.m.appearance.language;
    let items = Language::ALL.map(|l| {
        let name = match l {
            Language::Auto => tr!("settings.lang_auto"),
            Language::ZhCn => tr!("settings.lang_zh_cn"),
            Language::EnUs => tr!("settings.lang_en_us"),
            Language::JaJp => tr!("settings.lang_ja_jp"),
        };
        let is_sel = cur == l;
        let mut it = menu_item(name);
        if is_sel {
            it = it.icon(Icon::Check);
        }
        it.on_click(move |g: &mut Gui| {
            g.ui.dropdown_choice = (l != g.m.appearance.language).then_some(l);
            g.close_dropdown();
        })
    });

    menu().min_width(min_width.max(180.0)).children(items).into()
}
