//! Content library: mods, resource packs, shaders, saves and screenshots in one panel.
//! The search box filters local items. Mods, resource packs and shaders also search Modrinth.
//! Online results are listed under the local items.

use cairn_bridge::{
    ContentItem, ContentKind, MOD_CATEGORIES, OnlineFilter, OnlineItem, OnlineSort, OnlineSource, OnlineState, Remote, clean_zh_name, split_zh_name,
};
use cairn_i18n::tr;
use cairn_ui::{
    Align, El, Icon, Insets, Rect, Styled, button, checkbox, click_area, column, divider, icon, icon_button, label, list_item, panel, reveal, row, scroll, spinner,
    tab, tag, text_input,
};

use super::controls::action_slot;
use super::format;
use super::{empty_state, frame, header, loading_row, page_switch, removable, swap};
use crate::gui::{Confirm, DropdownKind, Gui, Removal, Sheet};
use crate::style::{RED, accent, accent_light, fg};
use crate::widgets::{initial, tile};

fn kind_icon(k: ContentKind) -> Icon {
    match k {
        ContentKind::Mods => Icon::Cube,
        ContentKind::ResourcePacks => Icon::Image,
        ContentKind::ShaderPacks => Icon::Sun,
        ContentKind::Saves => Icon::Globe,
        ContentKind::Screenshots => Icon::Camera,
    }
}

fn placeholder(k: ContentKind) -> &'static str {
    match k {
        ContentKind::Mods => tr!("library.placeholder_mods"),
        ContentKind::ResourcePacks => tr!("library.placeholder_resourcepacks"),
        ContentKind::ShaderPacks => tr!("library.placeholder_shaderpacks"),
        ContentKind::Saves => tr!("library.placeholder_saves"),
        ContentKind::Screenshots => tr!("library.placeholder_screenshots"),
    }
}

impl Gui {
    /// Switches the tab. The current tab fades out first, then the data changes
    /// and the new tab fades in (see [`Gui::arrive_tab`]).
    fn switch_tab(&mut self, k: ContentKind) {
        self.ui.tab.go(k);
        self.ui.confirm = None;
        self.close_categories();
    }

    /// Called when the old tab has faded out. The data changes only now, otherwise the fading page
    /// would show the new tab's items.
    fn arrive_tab(&mut self) {
        let Some(k) = self.ui.tab.arrive() else { return };
        self.ui.search.clear();
        self.ui.content_page_limit = 40;
        if let Some(id) = self.m.selected.clone() {
            self.bridge.open_content(&mut self.m, &id, k);
        }
    }
}

pub fn view(g: &Gui) -> El<Gui> {
    let Some(inst) = g.m.selected_instance() else {
        let body = column()
            .grow(1.0)
            .fill_width()
            .align(Align::Center)
            .child(empty_state(Icon::Package, tr!("library.empty_instance"), tr!("library.empty_instance_desc")))
            .child(
                button(tr!("library.install_game"))
                    .icon(Icon::Download)
                    .margin(Insets::new(0.0, 0.0, 32.0, 0.0))
                    .on_click(|g: &mut Gui| g.open(Sheet::NewInstance)),
            );
        return frame(header(label(tr!("library.title")).title(), Vec::new()), body.into(), None);
    };
    let kind = g.ui.tab.shown;
    let c = &g.m.content;

    let tabs = row()
        .gap(2.0)
        .children(ContentKind::ALL.map(|k| tab(k.label()).icon(kind_icon(k)).selected(k == g.ui.tab.target()).on_click(move |g: &mut Gui| g.switch_tab(k))));
    let mut actions: Vec<El<Gui>> = Vec::new();
    if kind == ContentKind::Mods && !inst.loaders.is_empty() {
        let check: El<Gui> = match c.updates {
            Remote::Loading => row().gap(8.0).child(spinner().shrink(0.0)).child(label(tr!("library.checking_updates")).caption().truncate()).into(),
            _ => button(tr!("library.check_updates")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.check_updates(&mut g.m)).into(),
        };
        actions.push(action_slot(160.0, Some(check)));
    }

    let filter_btn = kind.searchable_online().then(|| {
        let f = g.m.online.filter;
        let chosen = f.category.map(|i| MOD_CATEGORIES[i].label()).unwrap_or(tr!("library.all"));
        let label_text =
            if kind == ContentKind::Mods { format!("{} · {}", f.source.label(), chosen) } else { format!("{} · {}", f.source.label(), f.sort.label()) };
        let chevron = if g.ui.categories_open { Icon::ChevronUp } else { Icon::ChevronDown };
        tab(format!("{}: {label_text}", tr!("library.filter"))).icon(chevron).selected(g.ui.categories_open).on_click(|g: &mut Gui| g.toggle_categories())
    });

    let search = row()
        .fill_width()
        .padding(Insets::new(12.0, 18.0, 6.0, 18.0))
        .gap(10.0)
        .align(Align::Center)
        .child(text_input(&g.ui.search).icon(Icon::Search).placeholder(placeholder(kind)).grow(1.0).key(("search", kind.label())).on_change(
            |g: &mut Gui, s: String| {
                g.bridge.search_online(&mut g.m, &s);
                g.ui.search = s;
                g.ui.content_page_limit = 40;
                if let Remote::Ready(items) = &g.m.content.items {
                    let query = g.ui.search.trim().to_lowercase();
                    let matching: Vec<_> = items
                        .iter()
                        .filter(|it| {
                            query.is_empty()
                                || it.title.to_lowercase().contains(&query)
                                || it.file_name.to_lowercase().contains(&query)
                                || it.zh_name.as_deref().is_some_and(|zh| zh.to_lowercase().contains(&query))
                        })
                        .take(40)
                        .cloned()
                        .collect();
                    g.bridge.load_content_icons(&matching, g.ui.tab.shown);
                }
            },
        ))
        .children(filter_btn);

    let query = g.ui.search.trim().to_lowercase();
    let online_active = kind.searchable_online() && (!query.is_empty() || g.m.online.filter.category.is_some());
    let mut rows: Vec<El<Gui>> = Vec::new();
    if !g.m.manual.is_empty() {
        rows.push(manual_card(g));
    }
    let mut has_local = false;
    match &c.items {
        Remote::Ready(items) => {
            let shown: Vec<&ContentItem> = items
                .iter()
                .filter(|it| {
                    query.is_empty()
                        || it.title.to_lowercase().contains(&query)
                        || it.file_name.to_lowercase().contains(&query)
                        || it.zh_name.as_deref().is_some_and(|zh| zh.to_lowercase().contains(&query))
                })
                .collect();
            if shown.is_empty() && query.is_empty() {
                rows.push(empty_for(kind, inst.loaders.is_empty()));
            } else if shown.is_empty() {
                rows.push(label(tr!("library.no_matching_local")).caption().margin(Insets::new(4.0, 10.0, 2.0, 10.0)).into());
            } else {
                has_local = true;
            }
            let total = shown.len();
            let limit = g.ui.content_page_limit.max(40);
            let page = shown.iter().take(limit).copied();
            rows.extend(page.map(|it| removable(g, Removal::Content(it.path.clone()), local_row(g, it, kind))));

            if total > limit {
                let remaining = total - limit;
                let next_step = remaining.min(40);
                let current_limit = limit;
                rows.push(
                    row()
                        .fill_width()
                        .align(Align::Center)
                        .gap(10.0)
                        .padding(Insets::symmetric(10.0, 0.0))
                        .child(button(tr!("library.load_more", shown = current_limit, total = total, step = next_step)).icon(Icon::ChevronDown).on_click(
                            move |g: &mut Gui| {
                                let new_limit = current_limit + 40;
                                g.ui.content_page_limit = new_limit;
                                if let Remote::Ready(all_items) = &g.m.content.items {
                                    let slice: Vec<_> = all_items.iter().take(new_limit).cloned().collect();
                                    g.bridge.load_content_icons(&slice, kind);
                                }
                            },
                        ))
                        .child(button(tr!("library.expand_all")).on_click(move |g: &mut Gui| {
                            g.ui.content_page_limit = total;
                            if let Remote::Ready(all_items) = &g.m.content.items {
                                g.bridge.load_content_icons(all_items, kind);
                            }
                        }))
                        .into(),
                );
            }
        }
        Remote::Failed(e) => rows.push(label(tr!("library.read_failed", error = e)).caption().color(RED).margin(10.0).into()),
        _ => rows.push(loading_row(tr!("common.loading"))),
    }
    if online_active {
        rows.extend(online_rows(g, has_local));
    }

    let content = column()
        .grow(1.0)
        .min_height(0.0)
        .fill_width()
        .align(Align::Stretch)
        .child(search)
        .child(scroll().grow(1.0).min_height(0.0).align(Align::Stretch).padding(Insets::new(6.0, 12.0, 12.0, 12.0)).gap(2.0).children(rows));

    let mut body = column().grow(1.0).min_height(0.0).fill_width().align(Align::Stretch).child(page_switch(
        ("library_tab", kind),
        g.ui.tab.leaving(),
        |g: &mut Gui| g.arrive_tab(),
        content,
    ));

    if kind.searchable_online() && g.ui.categories_open {
        body = body.child(click_area().absolute().top(0.0).right(0.0).bottom(0.0).left(0.0).on_click(|g: &mut Gui| g.close_categories()));
    }
    if kind.searchable_online() && (g.ui.categories_open || g.ui.categories_leaving) {
        body = body.child(
            reveal()
                .from_above()
                .absolute()
                .top(52.0)
                .right(18.0)
                .width(400.0)
                .key("filter_popover")
                .leaving(!g.ui.categories_open)
                .on_left(|g: &mut Gui| g.ui.categories_leaving = false)
                .child(filter_dropdown(g, kind)),
        );
    }

    let (hint, hint_color) =
        if g.ui.drop_hover { (tr!("library.drop_hover", name = &inst.name), accent_light()) } else { (tr!("library.drop_hint").to_string(), fg(0.45)) };
    let footer = row()
        .fill_width()
        .padding(Insets::new(9.0, 12.0, 9.0, 20.0))
        .gap(10.0)
        .child(icon(Icon::Folder).icon_size(15.0).color(hint_color))
        .child(label(hint).font_size(13.0).color(hint_color).grow(1.0).min_width(0.0).truncate())
        .child(
            label(tr!("library.local_count", count = c.counts.get(&kind).map(|count| count.to_string()).unwrap_or_else(|| "—".into())))
                .caption()
                .width(120.0)
                .shrink(0.0)
                .truncate(),
        )
        .child(button(tr!("library.open_dir")).icon(Icon::Folder).on_click(|g: &mut Gui| g.bridge.open_content_folder(&mut g.m)));

    frame(header(tabs, actions), body.into(), Some(footer.into()))
}

fn empty_for(kind: ContentKind, vanilla: bool) -> El<Gui> {
    match kind {
        ContentKind::Mods if vanilla => empty_state(Icon::Cube, tr!("library.empty_vanilla"), tr!("library.empty_vanilla_desc")),
        ContentKind::Mods => empty_state(Icon::Cube, tr!("library.empty_mods"), tr!("library.empty_mods_desc")),
        ContentKind::ResourcePacks => empty_state(Icon::Image, tr!("library.empty_resourcepacks"), tr!("library.empty_resourcepacks_desc")),
        ContentKind::ShaderPacks => empty_state(Icon::Sun, tr!("library.empty_shaderpacks"), tr!("library.empty_shaderpacks_desc")),
        ContentKind::Saves => empty_state(Icon::Globe, tr!("library.empty_saves"), tr!("library.empty_saves_desc")),
        ContentKind::Screenshots => empty_state(Icon::Camera, tr!("library.empty_screenshots"), tr!("library.empty_screenshots_desc")),
    }
}

fn local_row(g: &Gui, it: &ContentItem, kind: ContentKind) -> El<Gui> {
    let path = it.path.clone();
    let busy = g.m.is_busy(&it.path);
    let update = g.m.content.updates.ready().and_then(|u| u.get(&it.path));
    let confirming = g.ui.confirm.as_ref() == Some(&Confirm::Content(it.path.clone()));
    let toggles = matches!(kind, ContentKind::Mods | ContentKind::ResourcePacks | ContentKind::ShaderPacks);

    let icon_img = g.icon(&it.path.to_string_lossy());
    let is_zh = cairn_i18n::is_chinese();
    let (clean_zh, orig_name) = if is_zh {
        match (&it.zh_name, &it.title) {
            (Some(zh), orig) => {
                let (zh_cleaned, extracted_zh) = split_zh_name(zh);
                let (orig_cleaned, extracted_orig) = split_zh_name(orig);
                let zh_final = zh_cleaned.or(orig_cleaned);
                let en_final = extracted_orig
                    .or(extracted_zh)
                    .or_else(|| if zh_final.as_deref() != Some(orig.as_str()) && !cairn_bridge::has_cjk(orig) { Some(orig.clone()) } else { None });
                (zh_final, en_final)
            }
            (None, orig) => {
                let (zh_cleaned, extracted) = split_zh_name(orig);
                let en_final = extracted.or_else(|| (!cairn_bridge::has_cjk(orig)).then(|| orig.clone()));
                (zh_cleaned, en_final)
            }
        }
    } else {
        let (_zh, extracted) = split_zh_name(&it.title);
        let en_title = extracted.unwrap_or_else(|| it.title.clone());
        (None, Some(en_title))
    };
    let shown_title = if is_zh { clean_zh.as_deref().unwrap_or(&it.title) } else { orig_name.as_deref().unwrap_or(&it.title) };
    let lead: El<Gui> = match kind {
        ContentKind::Saves => {
            let mut t = tile(&it.title, "", 34.0).icon(Icon::Globe);
            if let Some(img) = icon_img {
                t = t.image(img);
            }
            t.into()
        }
        ContentKind::Screenshots => {
            let mut t = tile(&it.title, "", 34.0).icon(Icon::Image);
            if let Some(img) = icon_img {
                t = t.image(img);
            }
            t.into()
        }
        _ => {
            let mut t = tile(&it.title, initial(shown_title), 34.0).muted(!it.enabled);
            if let Some(img) = icon_img {
                t = t.image(img);
            }
            t.into()
        }
    };
    let sub = match kind {
        ContentKind::Saves => tr!("library.modified_ago", time = format::ago(it.modified)),
        ContentKind::Screenshots => format!("{} · {}", format::date(it.modified), format::bytes(it.size)),
        _ => it.file_name.clone(),
    };
    let title_color = if it.enabled { fg(0.94) } else { fg(0.5) };
    let mut title = row().gap(8.0).min_width(0.0).child(label(shown_title).truncate().weight(700).color(title_color));
    if is_zh {
        if let (Some(zh), Some(en)) = (clean_zh.as_deref(), orig_name.as_deref()) {
            if zh != en {
                title = title.child(label(en).caption().truncate().margin(Insets::new(1.0, 0.0, 0.0, 0.0)));
            }
        }
    }
    if let Some(v) = &it.version {
        title = title.child(label(v).caption().margin(Insets::new(1.0, 0.0, 0.0, 0.0)));
    }

    let mut r = list_item().key(it.path.to_string_lossy().into_owned());
    if toggles {
        let p = path.clone();
        r = r.child(checkbox(it.enabled).enabled(!busy).on_change(move |g: &mut Gui, on| g.bridge.set_content_enabled(&mut g.m, &p, on)));
    }
    r = r.child(lead).child(column().grow(1.0).min_width(0.0).gap(1.0).child(title).child(label(sub).caption().truncate()));

    if confirming {
        let p = path.clone();
        let confirm = row()
            .gap(8.0)
            .child(
                label(match kind {
                    ContentKind::Saves => tr!("library.delete_confirm_save"),
                    ContentKind::Screenshots => tr!("library.delete_confirm_screenshot"),
                    _ => tr!("library.delete_confirm_file"),
                })
                .font_size(13.0)
                .color(RED),
            )
            .child(button(tr!("common.delete")).on_click(move |g: &mut Gui| {
                g.ui.confirm = None;
                g.remove_after_collapse(Removal::Content(p.clone()));
            }))
            .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.ui.confirm = None));
        return r.child(swap(("confirm", it.path.to_string_lossy().into_owned()), confirm)).into();
    }
    let mut update_area = row().width(if kind == ContentKind::Mods { 152.0 } else { 32.0 }).height(32.0).shrink(0.0).justify(cairn_ui::Justify::End).gap(8.0);
    if busy {
        update_area = update_area.child(spinner());
    } else if let Some(u) = update {
        let p = path.clone();
        update_area = update_area
            .child(label(format!("→ {}", u.new_version)).caption().truncate().grow(1.0).min_width(0.0))
            .child(button(tr!("common.update")).icon(Icon::Refresh).on_click(move |g: &mut Gui| g.bridge.apply_update(&mut g.m, &p)));
    }
    if toggles {
        r = r.child(update_area).child(action_slot(76.0, (!it.enabled).then(|| tag(tr!("library.disabled")).into())));
    } else {
        r = r.child(action_slot(32.0, busy.then(|| spinner().into())));
    }
    if is_zh {
        if let Some(url) = it.wiki_url.clone() {
            r = r.child(icon_button(Icon::ExternalLink).label(tr!("library.view_on_mcmod")).on_click(move |g: &mut Gui| g.bridge.open_url(&url)));
        }
    }
    if matches!(kind, ContentKind::Saves | ContentKind::Screenshots) {
        let p = path.clone();
        let text = if kind == ContentKind::Saves { tr!("common.open") } else { tr!("common.browse") };
        r = r.child(button(text).on_click(move |g: &mut Gui| g.bridge.open_path(&p)));
    }
    let p = path.clone();
    r.child(icon_button(Icon::Trash).label(tr!("common.delete")).enabled(!busy).on_click(move |g: &mut Gui| g.ui.confirm = Some(Confirm::Content(p.clone()))))
        .into()
}

/// Filter panel that pops up at the top right: platform source, sort order and mod category.
fn filter_dropdown(g: &Gui, kind: ContentKind) -> El<Gui> {
    let f = g.m.online.filter;
    let apply = move |g: &mut Gui, f: OnlineFilter| {
        let q = g.ui.search.clone();
        g.bridge.set_online_filter(&mut g.m, f, &q);
    };
    let source = super::controls::segmented(&OnlineSource::ALL.map(|s| (s, s.label())), f.source, move |g: &mut Gui, s| {
        apply(g, OnlineFilter { source: s, ..g.m.online.filter })
    });
    let sort = super::controls::segmented(&OnlineSort::ALL.map(|s| (s, s.label())), f.sort, move |g: &mut Gui, s| {
        let q = g.ui.search.clone();
        let f = OnlineFilter { sort: s, ..g.m.online.filter };
        g.bridge.set_online_filter(&mut g.m, f, &q);
    });

    let header_row = row()
        .fill_width()
        .align(Align::Center)
        .gap(8.0)
        .child(icon(Icon::Menu).icon_size(14.0).color(fg(0.7)))
        .child(label(tr!("library.filter_settings")).font_size(13.5).weight(700).color(fg(0.95)).grow(1.0))
        .child(icon_button(Icon::Close).label(tr!("library.filter_collapse")).on_click(|g: &mut Gui| g.close_categories()));

    let mut col = column()
        .fill_width()
        .padding(Insets::new(12.0, 14.0, 14.0, 14.0))
        .gap(10.0)
        .child(header_row)
        .child(divider().fill_width())
        .child(
            row().fill_width().align(Align::Center).child(label(tr!("library.online_platform")).caption().weight(700).color(fg(0.65)).grow(1.0)).child(source),
        )
        .child(row().fill_width().align(Align::Center).child(label(tr!("library.sort_by")).caption().weight(700).color(fg(0.65)).grow(1.0)).child(sort));

    if kind == ContentKind::Mods {
        let chip = |index: Option<usize>, name: &str| {
            let name = name.to_string();
            tab(name).selected(f.category == index).on_click(move |g: &mut Gui| {
                let q = g.ui.search.clone();
                let f = OnlineFilter { category: index, ..g.m.online.filter };
                g.bridge.set_online_filter(&mut g.m, f, &q);
            })
        };
        let mut chips: Vec<_> = vec![chip(None, tr!("library.all"))];
        chips.extend(MOD_CATEGORIES.iter().enumerate().map(|(i, c)| chip(Some(i), c.label())));

        col = col
            .child(divider().fill_width())
            .child(
                row()
                    .fill_width()
                    .align(Align::Center)
                    .child(icon(Icon::Package).icon_size(13.0).color(fg(0.55)))
                    .child(label(tr!("library.mod_categories")).caption().weight(700).color(fg(0.65)).margin(Insets::new(0.0, 0.0, 0.0, 4.0)))
                    .child(row().grow(1.0)),
            )
            .child(row().fill_width().wrap(true).gap(4.0).children(chips));
    }

    panel().popover().padding(0.0).gap(0.0).fill_width().child(col).into()
}

/// A file that must be downloaded by hand from the website, because the author does not allow
/// third-party distribution.
fn manual_card(g: &Gui) -> El<Gui> {
    let count = g.m.manual.len();
    let mut col = column().fill_width().gap(4.0).padding(Insets::new(10.0, 12.0, 10.0, 12.0)).child(
        row()
            .gap(8.0)
            .child(icon(Icon::Warning).icon_size(16.0).color(crate::style::AMBER))
            .child(label(tr!("library.manual_card_title", count = count)).weight(700).color(fg(0.9)).grow(1.0))
            .child(button(tr!("library.got_it")).on_click(|g: &mut Gui| g.bridge.dismiss_manual(&mut g.m))),
    );
    for item in g.m.manual.iter().take(8) {
        let (url, dir) = (item.url.clone(), item.target_dir.clone());
        col = col.child(
            row()
                .gap(8.0)
                .child(label(&item.name).truncate().color(fg(0.9)).grow(1.0))
                .child(label(&item.file_name).caption().truncate())
                .child(button(tr!("library.open_web")).icon(Icon::ExternalLink).on_click(move |g: &mut Gui| g.bridge.open_url(&url)))
                .child(button(tr!("library.target_dir")).icon(Icon::Folder).on_click(move |g: &mut Gui| g.bridge.open_path(&dir))),
        );
    }
    col.into()
}

fn online_rows(g: &Gui, has_local: bool) -> Vec<El<Gui>> {
    let top_pad = if has_local { 10.0 } else { 4.0 };
    let heading = row()
        .gap(8.0)
        .padding(Insets::new(top_pad, 10.0, 4.0, 10.0))
        .child(icon(Icon::Globe).icon_size(14.0).color(accent()))
        .child(label(tr!("library.online_results", source = g.m.online.filter.source.label())).font_size(13.0).weight(700).color(fg(0.7)));
    let mut out: Vec<El<Gui>> = vec![heading.into()];
    match &g.m.online.results {
        Remote::Ready(list) if list.is_empty() => out.push(label(tr!("library.no_online_results")).caption().margin(Insets::new(4.0, 10.0, 4.0, 10.0)).into()),
        Remote::Ready(list) => out.extend(list.iter().map(|it| online_row(g, it))),
        Remote::Failed(e) => out.push(label(tr!("library.search_failed", error = e)).caption().color(RED).margin(Insets::symmetric(8.0, 10.0)).into()),
        Remote::Loading | Remote::Idle => out.push(loading_row(&tr!("library.searching_online", source = g.m.online.filter.source.label()))),
    }
    out
}

fn online_row(g: &Gui, it: &OnlineItem) -> El<Gui> {
    let pid = it.project_id.clone();
    let source = it.source;
    let action: El<Gui> = match &it.state {
        OnlineState::Available => {
            button(tr!("library.install")).icon(Icon::Download).on_click(move |g: &mut Gui| g.bridge.install_online(&mut g.m, &pid)).into()
        }
        OnlineState::Installing(_) => row().gap(8.0).child(spinner()).child(label(tr!("library.installing")).caption()).into(),
        OnlineState::Installed => tag(tr!("library.installed")).accent().into(),
        OnlineState::Failed(_) => button(tr!("common.retry")).icon(Icon::Refresh).on_click(move |g: &mut Gui| g.bridge.install_online(&mut g.m, &pid)).into(),
    };
    // Show the Chinese name and summary from MC Wiki only in Simplified Chinese.
    let is_zh = cairn_i18n::is_chinese();
    let clean_zh = if is_zh { it.zh_name.as_deref().and_then(clean_zh_name) } else { None };
    let shown = clean_zh.as_deref().unwrap_or(&it.title);
    let mut sub = if is_zh { it.zh_summary.clone().unwrap_or_else(|| it.description.clone()) } else { it.description.clone() };
    if let OnlineState::Failed(e) = &it.state {
        sub = tr!("library.install_failed", error = e);
    }
    let mut title = row().gap(8.0).min_width(0.0).child(label(shown).truncate().weight(700).color(fg(0.94)));
    if clean_zh.is_some() {
        title = title.child(label(&it.title).caption().truncate().margin(Insets::new(1.0, 0.0, 0.0, 0.0)));
    }
    title = title.child(label(&it.author).caption().truncate().margin(Insets::new(1.0, 0.0, 0.0, 0.0)));
    let icon_img = it.icon_url.as_deref().and_then(|u| g.icon(u));
    let mut t = tile(&it.title, initial(shown), 34.0);
    if let Some(img) = icon_img {
        t = t.image(img);
    }
    let mut r = list_item()
        .key(("online", source.label(), it.project_id.clone()))
        .child(t)
        .child(column().grow(1.0).min_width(0.0).gap(1.0).child(title).child(label(sub).caption().truncate()))
        .child(label(tr!("library.downloads_count", count = format::count(it.downloads))).caption().shrink(0.0));
    let platform_url = it.platform_url();
    let wiki_url = it.wiki_url.clone();
    let drop_pid = it.project_id.clone();
    let is_open = matches!(&g.ui.dropdown, Some(d) if matches!(&d.kind, DropdownKind::OnlineLinks { project_id, .. } if project_id == &it.project_id));
    let mut link_btn = icon_button(Icon::ExternalLink)
        .on_click_rect(move |g: &mut Gui, rect: Rect| {
            g.open_dropdown(
                DropdownKind::OnlineLinks {
                    project_id: drop_pid.clone(),
                    source,
                    platform_url: platform_url.clone(),
                    wiki_url: wiki_url.clone(),
                },
                rect,
            );
        });
    if is_open {
        link_btn = link_btn.color(accent());
    }
    r = r.child(link_btn);
    // The action labels differ in width, so they sit in a fixed slot and the download count on the left
    // does not jump.
    r.child(action_slot(136.0, Some(action))).into()
}
