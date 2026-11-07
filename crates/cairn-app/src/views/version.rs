//! Instance settings: overview (name, icon, shortcut), launch options, loader and export.
//! Each launch option can follow the global setting.

use cairn_bridge::{ExportEntry, ExportRequest, GcPolicy, Instance, MemoryPolicy, Remote, WindowConfig};
use cairn_i18n::tr;
use cairn_icons::VersionIcon;
use cairn_ui::{Align, El, Icon, Insets, Styled, button, checkbox, column, icon_button, label, row, scroll, tab, text_input};

use super::controls::{java_list, overridable, segmented, setting_row};
use super::instance_menu::{delete_scope, open_dir};
use super::loaders::{PickTarget, instance_icon, loader_name, picker, plan_for};
use super::memory;
use super::{empty_state, frame, header, page_switch, section, swap};
use crate::gui::{Confirm, Gui, LoaderPick, Sheet, VersionPage};
use crate::style::{RED, fg};
use crate::widgets::version_tile;

fn page_name(p: VersionPage) -> &'static str {
    match p {
        VersionPage::Overview => tr!("version.overview"),
        VersionPage::Launch => tr!("version.launch_options"),
        VersionPage::Loader => tr!("version.loader"),
        VersionPage::Export => tr!("version.export"),
    }
}

const PAGES: [(VersionPage, Icon); 4] =
    [(VersionPage::Overview, Icon::Info), (VersionPage::Launch, Icon::Gear), (VersionPage::Loader, Icon::Cube), (VersionPage::Export, Icon::Package)];

pub fn view(g: &Gui) -> El<Gui> {
    let Some(inst) = g.ui.version_id.as_deref().and_then(|id| g.m.instance(id)) else {
        let back = row()
            .gap(4.0)
            .child(icon_button(Icon::ChevronLeft).label(tr!("common.back")).on_click(|g: &mut Gui| g.open(Sheet::Instances)))
            .child(label(tr!("version.title")).title());
        return frame(header(back, Vec::new()), empty_state(Icon::Gamepad, tr!("version.not_found"), tr!("version.not_found_desc")), None);
    };
    let page = g.ui.version_page.shown;
    let target = g.ui.version_page.target();
    let id = inst.id.clone();
    let title = row()
        .gap(10.0)
        .child(icon_button(Icon::ChevronLeft).label(tr!("common.back")).on_click(|g: &mut Gui| g.open(Sheet::Instances)))
        .child(version_tile::<Gui>(instance_icon(&g.m, inst), 32.0))
        .child(
            column()
                .width(190.0)
                .gap(1.0)
                .child(label(&inst.name).weight(700).color(fg(0.95)).truncate())
                .child(label(super::dock::describe(inst)).caption().truncate()),
        )
        .child(row().width(8.0));
    let tabs = row().gap(2.0).children(PAGES.map(|(p, ic)| tab(page_name(p)).icon(ic).selected(p == target).on_click(move |g: &mut Gui| g.set_version_page(p))));
    let lead = row().gap(0.0).child(title).child(tabs);
    let rows = match page {
        VersionPage::Overview => overview(g, inst),
        VersionPage::Launch => settings(g, inst),
        VersionPage::Loader => modify(g, inst),
        VersionPage::Export => export(g, inst),
    };
    let body = page_switch(
        ("version_page", id, page as u8),
        g.ui.version_page.leaving(),
        |g: &mut Gui| {
            g.ui.version_page.arrive();
        },
        scroll().grow(1.0).min_height(0.0).align(Align::Stretch).padding(Insets::new(10.0, 16.0, 20.0, 16.0)).gap(6.0).children(rows),
    );
    frame(header(lead, Vec::new()), body, None)
}

fn field(value: &str, placeholder: &str, width: f32, on_change: impl Fn(&mut Gui, String) + 'static) -> El<Gui> {
    text_input(value).placeholder(placeholder).width(width).on_change(on_change).into()
}

fn overview(g: &Gui, inst: &Instance) -> Vec<El<Gui>> {
    let id = inst.id.clone();
    let draft = &g.ui.draft;
    let shown_name = inst.settings.display_name.clone().unwrap_or_else(|| inst.name.clone());
    let shown_note = inst.settings.note.clone().unwrap_or_default();
    let dirty = draft.name.trim() != shown_name || draft.note.trim() != shown_note;
    let save = {
        let id = id.clone();
        button(tr!("version.save")).accent().enabled(dirty && !draft.name.trim().is_empty()).on_click(move |g: &mut Gui| {
            let (name, note) = (g.ui.draft.name.trim().to_string(), g.ui.draft.note.trim().to_string());
            g.edit_instance(&id, move |s| {
                s.display_name = (!name.is_empty()).then_some(name);
                s.note = (!note.is_empty()).then_some(note);
            });
        })
    };

    let icons = {
        let current = inst.settings.icon.as_deref().and_then(VersionIcon::from_key);
        let auto = {
            let id = id.clone();
            tab(tr!("version.icon_auto")).selected(current.is_none()).on_click(move |g: &mut Gui| g.edit_instance(&id, |s| s.icon = None))
        };
        let tile = |ic: VersionIcon| {
            let id = id.clone();
            version_tile::<Gui>(ic, 40.0)
                .selected(current == Some(ic))
                .on_click(move |g: &mut Gui| g.edit_instance(&id, move |s| s.icon = Some(ic.key().to_string())))
        };
        let (first, second) = VersionIcon::ALL.split_at(7);
        column()
            .gap(6.0)
            .child(row().gap(6.0).child(auto).children(first.iter().map(|ic| tile(*ic))))
            .child(row().gap(6.0).children(second.iter().map(|ic| tile(*ic))))
    };

    let loaders = if inst.loaders.is_empty() {
        tr!("version.no_loader").to_string()
    } else {
        inst.loaders.iter().map(|l| format!("{} {}", loader_name(l.kind), l.version)).collect::<Vec<_>>().join(" · ")
    };
    let layout = if inst.game_dir == inst.root { tr!("version.no_isolation") } else { tr!("version.with_isolation") }.to_string();

    let open = |ids: String, sub: &'static str, name: &'static str, ic: Icon| button(name).icon(ic).on_click(move |g: &mut Gui| open_dir(g, &ids, sub));
    let shortcuts = row()
        .gap(8.0)
        .child(open(id.clone(), "", tr!("version.game_folder"), Icon::Folder))
        .child(open(id.clone(), "saves", tr!("version.saves"), Icon::Globe))
        .child(open(id.clone(), "mods", tr!("version.mods"), Icon::Package))
        .child(open(id.clone(), "screenshots", tr!("version.screenshots"), Icon::Camera));

    let manage = {
        let (repair, del) = (id.clone(), id.clone());
        let mut r =
            row().gap(8.0).child(button(tr!("version.repair")).icon(Icon::Refresh).on_click(move |g: &mut Gui| g.bridge.repair_instance(&mut g.m, &repair)));
        if g.ui.confirm == Some(Confirm::Instance(id.clone())) {
            let go = id.clone();
            let confirm = row()
                .gap(8.0)
                .min_width(0.0)
                .shrink(1.0)
                .child(label(delete_scope(inst)).font_size(13.0).color(RED).shrink(1.0).min_width(0.0))
                .child(button(tr!("common.delete")).on_click(move |g: &mut Gui| {
                    g.ui.confirm = None;
                    g.bridge.delete_instance(&mut g.m, &go);
                    g.open(Sheet::Instances);
                }))
                .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.ui.confirm = None));
            r = r.child(swap("delete-confirm", confirm));
        } else {
            r = r.child(swap(
                "delete",
                button(tr!("version.delete_instance")).icon(Icon::Trash).on_click(move |g: &mut Gui| g.ui.confirm = Some(Confirm::Instance(del.clone()))),
            ));
        }
        r
    };

    let favorite = {
        let id = id.clone();
        checkbox(inst.settings.favorite).label(tr!("version.add_favorite")).on_change(move |g: &mut Gui, on| g.edit_instance(&id, move |s| s.favorite = on))
    };

    vec![
        section(tr!("version.personalization"), Icon::Gear),
        setting_row(
            tr!("version.name_label"),
            tr!("version.name_desc"),
            field(&draft.name, tr!("version.name_placeholder"), 300.0, |g, s| g.ui.draft.name = s),
        ),
        setting_row(
            tr!("version.desc_label"),
            tr!("version.desc_desc"),
            field(&draft.note, tr!("version.desc_placeholder"), 300.0, |g, s| g.ui.draft.note = s),
        ),
        {
            // Placed right under the two inputs and aligned to them, so it reads as saving both.
            let mut r = row()
                .fill_width()
                .justify(cairn_ui::Justify::End)
                .align(Align::Center)
                .gap(10.0)
                .padding(Insets::new(0.0, 6.0, 0.0, 6.0))
                .margin(Insets::new(-4.0, 0.0, 0.0, 0.0));
            if dirty {
                r = r.child(label(tr!("version.unsaved_hint")).caption());
            }
            r.child(save).into()
        },
        setting_row(tr!("version.icon_label"), tr!("version.icon_desc"), icons),
        setting_row(tr!("version.favorite_label"), "", favorite),
        section(tr!("version.info"), Icon::Info),
        setting_row(tr!("version.game_version"), &inst.mc_version.clone().unwrap_or_else(|| inst.version_id.clone()), row()),
        setting_row(tr!("version.loader_label"), &loaders, row()),
        setting_row(tr!("version.location_label"), &format!("{layout} · {}", inst.game_dir.display()), row()),
        section(tr!("version.shortcuts"), Icon::Folder),
        row().padding(Insets::symmetric(8.0, 4.0)).child(shortcuts).into(),
        section(tr!("version.advanced_management"), Icon::Chip),
        row().padding(Insets::symmetric(8.0, 4.0)).child(manage).into(),
    ]
}

fn gc_label(p: GcPolicy) -> &'static str {
    match p {
        GcPolicy::Auto => tr!("version.gc_auto"),
        GcPolicy::G1 => "G1",
        GcPolicy::Zgc => "ZGC",
        GcPolicy::None => tr!("version.gc_none"),
    }
}

fn mem_label(g: &Gui, p: MemoryPolicy) -> String {
    match p {
        MemoryPolicy::Auto => tr!("version.memory_auto").to_string(),
        MemoryPolicy::Fixed { mib } => {
            let _ = g;
            format!("{:.1} GB", mib as f64 / 1024.0)
        }
    }
}

fn settings(g: &Gui, inst: &Instance) -> Vec<El<Gui>> {
    let id = inst.id.clone();
    let s = &inst.settings;
    let launch = &g.m.config.launch;
    let mut rows: Vec<El<Gui>> = vec![label(tr!("version.settings_note")).caption().margin(Insets::new(0.0, 0.0, 6.0, 8.0)).into()];

    // Version isolation: when on, saves, mods and logs live in `versions/<version>/`.
    {
        let on = inst.settings.version_isolation.unwrap_or(inst.game_dir != inst.root);
        let a = id.clone();
        rows.push(section(tr!("version.isolation"), Icon::Folder));
        rows.push(setting_row(
            tr!("version.isolation"),
            tr!("version.isolation_desc"),
            segmented(&[(true, tr!("version.enable")), (false, tr!("version.disable"))], on, move |g: &mut Gui, on| {
                g.edit_instance(&a, move |s| s.version_isolation = Some(on))
            }),
        ));
    }

    rows.push(section("Java", Icon::Cup));
    let global_java = match &g.m.config.java.preferred {
        Some(p) => p.display().to_string(),
        None => tr!("version.auto_java").to_string(),
    };
    let (a, b) = (id.clone(), id.clone());
    let chosen = s.java_path.clone();
    rows.push(overridable(
        tr!("version.used_java"),
        "",
        s.java_path.is_some(),
        global_java,
        move |g: &mut Gui, own| {
            let first = g.m.javas.ready().and_then(|l| l.first()).map(|j| j.path.clone());
            g.edit_instance(&a, move |s| s.java_path = if own { first } else { None });
        },
        move || java_picker(g, &b, chosen),
    ));

    rows.push(section(tr!("version.memory"), Icon::Chip));
    let (a, b) = (id.clone(), id.clone());
    let global_mem = mem_label(g, launch.memory);
    let own_mem = s.memory;
    rows.push(overridable(
        tr!("version.game_memory"),
        "",
        own_mem.is_some(),
        global_mem,
        move |g: &mut Gui, own| {
            let start = g.m.config.launch.memory;
            g.edit_instance(&a, move |s| s.memory = own.then_some(start));
        },
        move || memory::block(g, own_mem.unwrap_or_default(), Some(&b.clone()), move |g: &mut Gui, p| g.edit_instance(&b, move |s| s.memory = Some(p))),
    ));

    rows.push(section(tr!("version.advanced"), Icon::Gear));
    let (a, b) = (id.clone(), id.clone());
    rows.push(overridable(
        tr!("version.gc_label"),
        tr!("version.gc_desc"),
        s.gc.is_some(),
        gc_label(launch.gc),
        move |g: &mut Gui, own| {
            let start = g.m.config.launch.gc;
            g.edit_instance(&a, move |s| s.gc = own.then_some(start));
        },
        move || {
            segmented(
                &[(GcPolicy::Auto, tr!("version.gc_auto")), (GcPolicy::G1, "G1"), (GcPolicy::Zgc, "ZGC"), (GcPolicy::None, tr!("version.gc_none"))],
                s.gc.unwrap_or_default(),
                move |g: &mut Gui, p| g.edit_instance(&b, move |s| s.gc = Some(p)),
            )
        },
    ));
    let (a, b) = (id.clone(), id.clone());
    let jvm = s.extra_jvm_args.clone().unwrap_or_default();
    rows.push(overridable(
        tr!("version.jvm_args"),
        tr!("version.jvm_args_desc"),
        s.extra_jvm_args.is_some(),
        if launch.extra_jvm_args.trim().is_empty() { tr!("version.empty_param").to_string() } else { launch.extra_jvm_args.clone() },
        move |g: &mut Gui, own| {
            let start = g.m.config.launch.extra_jvm_args.clone();
            g.edit_instance(&a, move |s| s.extra_jvm_args = own.then_some(start));
        },
        move || {
            text_input(&jvm)
                .placeholder(tr!("version.jvm_placeholder"))
                .on_change(move |g: &mut Gui, t: String| g.edit_instance(&b, move |s| s.extra_jvm_args = Some(t)))
                .into()
        },
    ));
    let (a, b) = (id.clone(), id.clone());
    let args = s.extra_game_args.clone().unwrap_or_default();
    rows.push(overridable(
        tr!("version.game_args"),
        tr!("version.game_args_desc"),
        s.extra_game_args.is_some(),
        if launch.extra_game_args.trim().is_empty() { tr!("version.empty_param").to_string() } else { launch.extra_game_args.clone() },
        move |g: &mut Gui, own| {
            let start = g.m.config.launch.extra_game_args.clone();
            g.edit_instance(&a, move |s| s.extra_game_args = own.then_some(start));
        },
        move || {
            text_input(&args)
                .placeholder(tr!("version.game_args_placeholder"))
                .on_change(move |g: &mut Gui, t: String| g.edit_instance(&b, move |s| s.extra_game_args = Some(t)))
                .into()
        },
    ));

    rows.push(section(tr!("version.window_and_server"), Icon::Gamepad));
    let (a, b) = (id.clone(), id.clone());
    let global_window = window_label(&launch.window);
    let own_window = s.window.clone();
    rows.push(overridable(
        tr!("version.game_window"),
        "",
        own_window.is_some(),
        global_window,
        move |g: &mut Gui, own| {
            let start = g.m.config.launch.window.clone();
            g.edit_instance(&a, move |s| s.window = own.then_some(start));
        },
        move || {
            window_editor(own_window.unwrap_or_default(), move |g: &mut Gui, f| {
                g.edit_instance(&b, move |s| s.window = Some(f(s.window.take().unwrap_or_default())))
            })
        },
    ));
    let b = id.clone();
    rows.push(setting_row(
        tr!("version.auto_join_server"),
        tr!("version.auto_join_desc"),
        field(s.server.as_deref().unwrap_or(""), tr!("version.server_placeholder"), 260.0, move |g, t| {
            g.edit_instance(&b, move |s| s.server = (!t.trim().is_empty()).then(|| t.trim().to_string()))
        }),
    ));
    rows
}

pub fn window_label(w: &WindowConfig) -> String {
    if w.fullscreen { tr!("version.fullscreen").to_string() } else { format!("{} × {}", w.width, w.height) }
}

pub type WindowEdit = Box<dyn FnOnce(WindowConfig) -> WindowConfig>;

type EditWindow = std::rc::Rc<dyn Fn(&mut Gui, WindowEdit)>;

pub fn window_editor(w: WindowConfig, edit: impl Fn(&mut Gui, WindowEdit) + 'static) -> El<Gui> {
    let edit = std::rc::Rc::new(edit);
    let number = |edit: EditWindow, value: u32, width: bool| {
        text_input(value.to_string()).width(78.0).on_change(move |g: &mut Gui, t: String| {
            if let Ok(n) = t.trim().parse::<u32>() {
                edit(
                    g,
                    Box::new(move |mut w| {
                        if width {
                            w.width = n.clamp(200, 16384);
                        } else {
                            w.height = n.clamp(200, 16384);
                        }
                        w
                    }),
                );
            }
        })
    };
    let fullscreen = {
        let edit = edit.clone();
        checkbox(w.fullscreen).label(tr!("version.fullscreen")).on_change(move |g: &mut Gui, on| {
            edit(
                g,
                Box::new(move |mut w| {
                    w.fullscreen = on;
                    w
                }),
            )
        })
    };
    row().gap(10.0).child(number(edit.clone(), w.width, true)).child(label("×").caption()).child(number(edit, w.height, false)).child(fullscreen).into()
}

fn java_picker(g: &Gui, id: &str, chosen: Option<std::path::PathBuf>) -> El<Gui> {
    let id = id.to_string();
    column()
        .fill_width()
        .gap(2.0)
        .align(Align::Stretch)
        .children(java_list(g, chosen.as_deref(), move |g: &mut Gui, p| g.edit_instance(&id, move |s| s.java_path = Some(p))))
        .child(
            row()
                .gap(8.0)
                .padding(Insets::new(6.0, 0.0, 0.0, 4.0))
                .child(button(tr!("version.add_java_btn")).icon(Icon::Plus).on_click(|g: &mut Gui| g.bridge.pick_java(&mut g.m)))
                .child(button(tr!("version.rescan")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.load_javas(&mut g.m, true))),
        )
        .into()
}

fn modify(g: &Gui, inst: &Instance) -> Vec<El<Gui>> {
    let Some(mc) = inst.mc_version.clone() else {
        return vec![empty_state(Icon::Warning, tr!("version.cannot_determine_version"), tr!("version.cannot_determine_version_desc"))];
    };
    let current = LoaderPick::of_instance(inst);
    let pick = &g.ui.modify;
    let plan = plan_for(g, PickTarget::Modify, &mc);
    let current_version = current.loader_version.exact().map(str::to_string);
    let target = plan.loader.as_ref().map(|(k, v)| (*k, v.clone()));
    // Switching to 'auto' for the same loader is not a change, unless auto picks a different version.
    let changed = pick.loader != current.loader || target.as_ref().is_some_and(|(_, v)| Some(v) != current_version.as_ref());
    let ready = pick.loader.is_none() || plan.problem.is_none();
    let now = match current.loader {
        Some(k) => format!("{} {}", loader_name(k), current_version.clone().unwrap_or_default()),
        None => tr!("version.no_loader").to_string(),
    };
    let note = if inst.version_id == mc { tr!("version.modify_note_vanilla") } else { tr!("version.modify_note_loader") };
    let id = inst.id.clone();
    let loader = plan.spec();
    let apply = button(if pick.loader.is_none() { tr!("version.revert_vanilla") } else { tr!("version.apply_changes") })
        .accent()
        .icon(Icon::Download)
        .enabled(changed && ready)
        .on_click(move |g: &mut Gui| {
            let loader = loader.clone();
            g.bridge.change_loader(&mut g.m, &id, loader);
            g.close();
            g.ui.tasks_open = true;
            g.ui.tasks_leaving = false;
        });
    vec![
        section(tr!("version.current_section"), Icon::Info),
        setting_row(&format!("Minecraft {mc}"), &now, row()),
        section(tr!("version.change_section"), Icon::Cube),
        picker(g, PickTarget::Modify, &mc, false),
        label(note).caption().margin(Insets::new(6.0, 8.0, 0.0, 8.0)).into(),
        row().fill_width().justify(cairn_ui::Justify::End).padding(Insets::new(4.0, 6.0, 0.0, 6.0)).child(apply).into(),
    ]
}

fn export(g: &Gui, inst: &Instance) -> Vec<El<Gui>> {
    let form = &g.ui.export;
    let id = inst.id.clone();
    let mut rows: Vec<El<Gui>> = vec![
        section(tr!("version.export_modpack_info"), Icon::Package),
        setting_row(tr!("version.modpack_name"), "", field(&form.name, tr!("version.modpack_name_placeholder"), 240.0, |g, s| g.ui.export.name = s)),
        setting_row(
            tr!("version.modpack_version"),
            "",
            field(&form.version, tr!("version.modpack_version_placeholder"), 160.0, |g, s| g.ui.export.version = s),
        ),
        setting_row(
            tr!("version.modpack_summary"),
            "",
            field(&form.summary, tr!("version.modpack_summary_placeholder"), 300.0, |g, s| g.ui.export.summary = s),
        ),
        section(tr!("version.export_included"), Icon::Folder),
    ];
    let entries: Vec<ExportEntry> = match &g.m.export {
        Remote::Ready(list) => list.clone(),
        Remote::Failed(e) => {
            rows.push(label(e.as_str()).caption().color(RED).into());
            Vec::new()
        }
        _ => {
            rows.push(label(tr!("version.checking_export_content")).caption().margin(Insets::symmetric(8.0, 4.0)).into());
            Vec::new()
        }
    };
    let selected = |e: &ExportEntry| match &form.include {
        Some(set) => set.contains(e.path),
        None => e.default && e.exists,
    };
    for e in &entries {
        let path = e.path;
        let all: Vec<ExportEntry> = entries.clone();
        rows.push(
            row()
                .padding(Insets::symmetric(8.0, 3.0))
                .child(
                    checkbox(selected(e) && e.exists)
                        .enabled(e.exists)
                        .label(if e.exists { e.label.to_string() } else { format!("{}{}", e.label, tr!("version.not_present")) })
                        .on_change(move |g: &mut Gui, on| {
                            let mut set =
                                g.ui.export
                                    .include
                                    .take()
                                    .unwrap_or_else(|| all.iter().filter(|x| x.default && x.exists).map(|x| x.path.to_string()).collect());
                            if on {
                                set.insert(path.to_string());
                            } else {
                                set.remove(path);
                            }
                            g.ui.export.include = Some(set);
                        }),
                )
                .into(),
        );
    }
    let ready = !form.name.trim().is_empty() && !form.version.trim().is_empty() && matches!(g.m.export, Remote::Ready(_));
    let include: Vec<String> = entries.iter().filter(|e| selected(e) && e.exists).map(|e| e.path.to_string()).collect();
    rows.push(label(tr!("version.export_notice")).caption().margin(Insets::new(8.0, 8.0, 0.0, 8.0)).into());
    rows.push(
        row()
            .fill_width()
            .justify(cairn_ui::Justify::End)
            .padding(Insets::new(4.0, 6.0, 0.0, 6.0))
            .child(button(tr!("version.export_button")).accent().icon(Icon::ExternalLink).enabled(ready && !include.is_empty()).on_click(move |g: &mut Gui| {
                let f = &g.ui.export;
                let summary = f.summary.trim();
                let req = ExportRequest {
                    name: f.name.trim().to_string(),
                    version: f.version.trim().to_string(),
                    summary: (!summary.is_empty()).then(|| summary.to_string()),
                    include: include.clone(),
                };
                g.bridge.export_pack(&mut g.m, &id, req);
            }))
            .into(),
    );
    rows
}
