//! First-run wizard (Terms of Service and Privacy Policy, with consent), plus batch import of game versions from an existing folder.

use cairn_i18n::tr;

use cairn_bridge::{ImportCandidate, Remote};
use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, button, checkbox, column, icon, icon_button, label, list_item, row, scroll, tag};

use super::loaders::version_icon;
use super::{empty_state, frame, header, legal, loading_row, section};
use crate::gui::{Gui, Sheet};
use crate::style::{GREEN, RED, fg};
use crate::widgets::version_tile;

impl Gui {
    pub fn is_candidate_selected(&self, id: &str) -> bool {
        match &self.ui.import_selected {
            Some(set) => set.contains(id),
            None => true, // Selected by default.
        }
    }

    pub fn toggle_candidate_select(&mut self, id: &str) {
        let mut set = self.ui.import_selected.take().unwrap_or_else(|| {
            if let Remote::Ready(ref list) = self.m.import_candidates { list.iter().map(|c| c.id.clone()).collect() } else { std::collections::HashSet::new() }
        });
        if set.contains(id) {
            set.remove(id);
        } else {
            set.insert(id.to_string());
        }
        self.ui.import_selected = Some(set);
    }

    pub fn skip_setup_import(&mut self) {
        self.bridge.complete_setup(&mut self.m);
        self.ui.setup_finished = true;
        self.ui.sheet = None;
    }

    pub fn select_all_candidates(&mut self, list: &[ImportCandidate]) {
        self.ui.import_selected = Some(list.iter().map(|c| c.id.clone()).collect());
    }

    pub fn deselect_all_candidates(&mut self) {
        self.ui.import_selected = Some(std::collections::HashSet::new());
    }

    pub fn do_import_selected(&mut self, list: &[ImportCandidate]) {
        let selected: Vec<ImportCandidate> = list.iter().filter(|c| self.is_candidate_selected(&c.id)).cloned().collect();
        if selected.is_empty() {
            return;
        }
        self.bridge.import_candidates(&mut self.m, selected);
        self.bridge.complete_setup(&mut self.m);
        self.ui.setup_finished = true;
        self.ui.sheet = None;
    }
}

pub fn initial_setup_view(g: &Gui) -> El<Gui> {
    if !g.m.config.eula_accepted { eula_view(g) } else { import_flow_view(g, true) }
}

/// Import panel opened by hand from settings.
pub fn import_versions_view(g: &Gui) -> El<Gui> {
    import_flow_view(g, false)
}

/// Standalone panel for the Terms of Service and the Privacy Policy, opened from settings. Unlike the
/// wizard, it has no consent controls and no import step.
pub fn eula_panel_view(g: &Gui) -> El<Gui> {
    let _ = g;
    let head = header(label(tr!("setup.user_agreement")).title(), vec![]);

    let links = row()
        .fill_width()
        .gap(10.0)
        .justify(Justify::End)
        .padding(Insets::new(10.0, 18.0, 12.0, 18.0))
        .child(button(tr!("setup.mojang_rules")).icon(Icon::ExternalLink).on_click(|g: &mut Gui| g.bridge.open_url("https://www.minecraft.net/eula")))
        .child(
            button(tr!("setup.minecraft_commercial"))
                .icon(Icon::ExternalLink)
                .on_click(|g: &mut Gui| g.bridge.open_url("https://www.minecraft.net/commercial-usage-guidelines")),
        )
        .child(button(tr!("common.close")).accent().on_click(|g: &mut Gui| g.close_current()));

    frame(head, legal_scroll(false), Some(links.into()))
}

/// The scrolling text of both documents. With `gate` set, reaching its end is recorded in `Ui::legal_read`,
/// which the consent controls of the first-run wizard wait for.
fn legal_scroll(gate: bool) -> El<Gui> {
    let area = scroll().grow(1.0).align(Align::Stretch).padding(Insets::new(10.0, 16.0, 16.0, 16.0)).gap(8.0).children(legal::document());
    if gate {
        area.on_end(|g: &mut Gui, at_end| {
            if at_end {
                g.ui.legal_read = true;
            }
        })
        .into()
    } else {
        area.into()
    }
}

fn eula_view(g: &Gui) -> El<Gui> {
    let head = header(label(tr!("setup.welcome")).title(), vec![]);
    // The consent controls stay locked until the text has been scrolled to its end.
    let read = g.ui.legal_read;

    let intro = row()
        .fill_width()
        .gap(8.0)
        .padding(Insets::new(10.0, 18.0, 2.0, 18.0))
        .child(icon(Icon::Info).icon_size(16.0).color(fg(0.7)))
        .child(label(tr!("setup.legal_intro")).caption().color(fg(0.78)).grow(1.0).min_width(0.0));

    let status = if read {
        row().gap(6.0).child(icon(Icon::Check).icon_size(14.0).color(GREEN)).child(label(tr!("setup.legal_read_done")).caption().color(GREEN))
    } else {
        row().gap(6.0).child(icon(Icon::ChevronDown).icon_size(14.0).color(fg(0.65))).child(label(tr!("setup.legal_scroll_hint")).caption().color(fg(0.7)))
    };

    let agree = checkbox(read && g.ui.setup_eula_agreed).label(tr!("setup.agreement_agree_check")).enabled(read).on_change(|g: &mut Gui, val| {
        if g.ui.legal_read {
            g.ui.setup_eula_agreed = val;
        }
    });

    let confirm = button(tr!("setup.legal_agree_button")).accent().enabled(read && g.ui.setup_eula_agreed).on_click(|g: &mut Gui| {
        if g.ui.legal_read && g.ui.setup_eula_agreed {
            g.bridge.accept_eula(&mut g.m);
        }
    });

    let footer = column()
        .fill_width()
        .gap(8.0)
        .padding(Insets::new(10.0, 18.0, 12.0, 18.0))
        .child(status)
        .child(row().fill_width().gap(10.0).child(row().grow(1.0).min_width(0.0).child(agree)).child(confirm))
        .child(label(tr!("setup.legal_decline_hint")).caption().color(fg(0.55)));

    let body = column().fill_width().align(Align::Stretch).child(intro).child(legal_scroll(true));

    frame(head, body.into(), Some(footer.into()))
}

fn import_flow_view(g: &Gui, is_first_run: bool) -> El<Gui> {
    let title_text = if is_first_run { tr!("setup.import_title") } else { tr!("settings.import_external") };
    let lead = row().gap(4.0);
    let lead = if is_first_run {
        lead
    } else {
        lead.child(icon_button(Icon::ChevronLeft).label(tr!("common.back")).on_click(|g: &mut Gui| g.open(Sheet::Settings)))
    };
    let head = header(lead.child(label(title_text).title()), vec![]);

    let folder_desc = match (&g.m.import_folder, &g.m.import_candidates) {
        (Some(p), _) => tr!("setup.current_scan_path", path = p.display()),
        (None, Remote::Idle | Remote::Loading) => tr!("setup.scanning").to_string(),
        (None, Remote::Ready(list)) if !list.is_empty() => tr!("setup.import_desc").to_string(),
        (None, _) => String::new(),
    };

    let folder_bar = row()
        .fill_width()
        .gap(8.0)
        .padding(Insets::new(4.0, 4.0, 8.0, 4.0))
        .child(label(folder_desc).caption().grow(1.0).truncate())
        .child(button(tr!("common.browse")).icon(Icon::Folder).on_click(|g: &mut Gui| {
            g.ui.import_selected = None;
            g.bridge.pick_import_folder(&mut g.m);
        }))
        .child(button(tr!("common.refresh")).icon(Icon::Refresh).on_click(|g: &mut Gui| {
            g.ui.import_selected = None;
            let folder = g.m.import_folder.clone();
            g.bridge.scan_import_candidates(&mut g.m, folder);
        }));

    let mut body_items = vec![folder_bar.into()];

    match &g.m.import_candidates {
        Remote::Idle | Remote::Loading => {
            body_items.push(loading_row(tr!("setup.scanning")));
        }
        Remote::Failed(e) => {
            body_items.push(label(tr!("library.search_failed", error = e)).caption().color(RED).into());
        }
        Remote::Ready(list) => {
            if list.is_empty() {
                body_items.push(empty_state(Icon::Folder, tr!("setup.no_candidates_title"), tr!("setup.no_candidates_desc")).into());
            } else {
                body_items.push(section(&tr!("setup.detected_versions", count = list.len()), Icon::Cube));

                // All candidates start ticked when the list first loads.
                let all_selected = match &g.ui.import_selected {
                    Some(set) => !list.is_empty() && list.iter().all(|c| set.contains(&c.id)),
                    None => !list.is_empty(),
                };

                for c in list {
                    body_items.push(candidate_row(g, c));
                }

                let list_clone1 = list.clone();
                let list_clone2 = list.clone();

                let selected_count = list.iter().filter(|c| g.is_candidate_selected(&c.id)).count();

                // Only one next-step button uses the accent color. It reads 'Import' when versions are picked and
                // 'Skip import' when none are.
                let mut import = button(tr!("setup.import_selected_count", count = selected_count))
                    .width(if cairn_i18n::current_language().resolve() == cairn_i18n::Language::JaJp { 360.0 } else { 224.0 })
                    .icon(Icon::Download)
                    .enabled(selected_count > 0)
                    .on_click(move |g: &mut Gui| g.do_import_selected(&list_clone2));
                if selected_count > 0 {
                    import = import.accent();
                }
                let action_bar = row()
                    .fill_width()
                    .gap(10.0)
                    .padding(Insets::symmetric(10.0, 4.0))
                    .child(
                        button(if all_selected { tr!("setup.deselect_all") } else { tr!("setup.select_all") })
                            .width(if cairn_i18n::current_language().resolve() == cairn_i18n::Language::JaJp { 168.0 } else { 112.0 })
                            .on_click(move |g: &mut Gui| {
                                if all_selected {
                                    g.deselect_all_candidates();
                                } else {
                                    g.select_all_candidates(&list_clone1);
                                }
                            }),
                    )
                    .child(row().grow(1.0))
                    .child(import);

                if is_first_run {
                    let mut skip = button(tr!("setup.skip_import")).on_click(|g: &mut Gui| g.skip_setup_import());
                    if selected_count == 0 {
                        skip = skip.accent();
                    }
                    body_items
                        .push(row().fill_width().justify(Justify::SpaceBetween).padding(Insets::new(8.0, 4.0, 4.0, 4.0)).child(skip).child(action_bar).into());
                } else {
                    body_items.push(action_bar.into());
                }
            }
        }
    }

    if is_first_run && matches!(g.m.import_candidates, Remote::Ready(ref l) if l.is_empty()) {
        body_items.push(
            row()
                .justify(Justify::End)
                .padding(Insets::symmetric(12.0, 4.0))
                .child(button(tr!("setup.finish_and_enter")).accent().on_click(|g: &mut Gui| g.skip_setup_import()))
                .into(),
        );
    }

    let body = scroll().grow(1.0).align(Align::Stretch).padding(Insets::new(8.0, 14.0, 16.0, 14.0)).gap(4.0).children(body_items);

    frame(head, body.into(), None)
}

fn candidate_row(g: &Gui, c: &ImportCandidate) -> El<Gui> {
    let id = c.id.clone();
    let checked = g.is_candidate_selected(&c.id);
    let icon = version_icon(&g.m, c.mc_version.as_deref().unwrap_or(&c.id), &c.loaders, c.settings.icon.as_deref(), c.problem.is_some());

    let mut subtitle_parts = Vec::new();
    if let Some(mc) = &c.mc_version {
        subtitle_parts.push(format!("{} {mc}", tr!("loaders.vanilla")));
    }
    for l in &c.loaders {
        subtitle_parts.push(format!("{:?} {}", l.kind, l.version));
    }
    subtitle_parts.push(c.root_name.clone());
    let sub = subtitle_parts.join(" · ");

    let mut badge_els = Vec::new();
    badge_els.push(tag(c.source.label()));

    for b in &c.badges {
        let mut t = tag(b);
        if b.contains("隔离") || b.contains("配置") || b.contains("设置") || b.contains("isolate") || b.contains("isolate") {
            t = t.accent();
        }
        badge_els.push(t);
    }

    let check_id = id.clone();
    let toggle_id = id.clone();

    list_item()
        .key(id)
        .on_click(move |g: &mut Gui| g.toggle_candidate_select(&toggle_id))
        .child(checkbox(checked).on_change(move |g: &mut Gui, _| {
            g.toggle_candidate_select(&check_id);
        }))
        .child(version_tile::<Gui>(icon, 36.0))
        .child(column().grow(1.0).min_width(0.0).gap(2.0).child(label(&c.id).truncate().weight(700).color(fg(0.95))).child(label(sub).caption().truncate()))
        .child(row().gap(4.0).children(badge_els))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_bridge::{Appearance, Bridge, Language, Sink};
    use cairn_ui::testing::Headless;
    use cairn_ui::{Role, ScrollDelta};
    use std::sync::Arc;
    use std::time::Duration;

    fn disabled(app: &Headless<Gui>, text: &str) -> bool {
        app.find(text).unwrap_or_else(|| panic!("找不到 {text}")).semantics.disabled
    }

    #[test]
    fn consent_stays_locked_until_the_text_is_scrolled_to_the_end() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            cairn_i18n::with_language(language, || {
                let sink: Sink = Arc::new(|_| true);
                let gui = Gui::new(Bridge::start(rt.handle().clone(), sink));
                let theme = crate::style::theme(&Appearance::default(), None, false);
                let mut app = Headless::with_theme(gui, |g: &Gui| initial_setup_view(g), 900.0, 700.0, 1.0, theme, text.take().unwrap());
                app.render();

                let (check, agree) = (tr!("setup.agreement_agree_check"), tr!("setup.legal_agree_button"));
                assert!(disabled(&app, check) && disabled(&app, agree), "{language:?}: 没读到文末前，同意控件应被锁住");
                // Clicking the locked checkbox does nothing.
                let at = app.find(check).unwrap().bounds.center();
                app.click(at);
                assert!(!app.state().ui.setup_eula_agreed, "{language:?}");

                // Scrolling to the end unlocks the checkbox, but the button still waits for the tick.
                let area = app.find_role(Role::ScrollArea)[0].bounds;
                app.pointer_move(area.center());
                app.wheel(ScrollDelta::Pixels { x: 0.0, y: -1.0e7 });
                app.advance(Duration::from_millis(50));
                app.render();
                assert!(app.state().ui.legal_read, "{language:?}: 滚到底应记下已读");
                assert!(!disabled(&app, check) && disabled(&app, agree), "{language:?}");
                assert!(app.find(tr!("setup.legal_end")).is_some(), "{language:?}: 应该看得到文末标记");

                let at = app.find(check).unwrap().bounds.center();
                app.click(at);
                app.render();
                assert!(app.state().ui.setup_eula_agreed && !disabled(&app, agree), "{language:?}");

                let at = app.find(agree).unwrap().bounds.center();
                app.click(at);
                assert!(app.state().m.config.eula_accepted, "{language:?}: 点同意后应记录已同意");
                text = Some(app.into_text_system());
            });
        }
    }
}
