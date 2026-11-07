//! The dock at the bottom: account, current instance, content library, settings and launch.
//! A borderless floating glass bar.

use cairn_i18n::tr;

use cairn_bridge::{Instance, Status};
use cairn_icons::VersionIcon;
use cairn_ui::{El, Icon, Insets, Styled, column, divider, label, list_item, panel, tab};

use super::loaders::{instance_icon, loader_name};
use crate::gui::{Gui, Sheet};
use crate::style::fg;
use crate::widgets::{LaunchButton, LaunchState, initial, launch_button, tile, version_tile};

pub fn dock(g: &Gui) -> El<Gui> {
    let open = &g.active_sheet().cloned();
    let sep = || divider().vertical().height(30.0).margin(Insets::symmetric(4.0, 0.0));

    let custom_tab = g.m.panels.first().map(|p| {
        let pid = p.id.clone();
        let is_selected = matches!(open, Some(Sheet::CustomPanel(id)) if id == &pid);
        tab(&p.title)
            .icon(Icon::Globe)
            .selected(is_selected)
            .capsule()
            .padding(Insets::symmetric(14.0, 10.0))
            .on_click(move |g: &mut Gui| g.toggle(Sheet::CustomPanel(pid.clone())))
    });

    let mut p = panel()
        .bar()
        .horizontal()
        .hover_effect(false)
        .padding(Insets::new(9.0, 9.0, 9.0, 9.0))
        .gap(6.0)
        .child(account(g))
        .child(sep())
        .child(instance_picker(g))
        .child(sep())
        .child(
            tab(tr!("dock.library"))
                .icon(Icon::Package)
                .selected(matches!(open, Some(Sheet::Library)))
                .capsule()
                .padding(Insets::symmetric(14.0, 10.0))
                .on_click(|g: &mut Gui| g.toggle(Sheet::Library)),
        );

    if let Some(ct) = custom_tab {
        p = p.child(ct);
    }

    p.child(
        tab("")
            .icon(Icon::Gear)
            .selected(matches!(open, Some(Sheet::Settings)))
            .capsule()
            .padding(Insets::symmetric(11.0, 10.0))
            .on_click(|g: &mut Gui| g.toggle(Sheet::Settings)),
    )
    .child(sep())
    .child(launch(g))
    .into()
}

fn account(g: &Gui) -> El<Gui> {
    let avatar = match g.m.selected_account() {
        Some(a) => {
            let mut t = tile(&a.name, initial(&a.name), 38.0).round();
            if let Some(img) = g.icon(&format!("avatar:{}", a.id)) {
                t = t.image(img);
            }
            t
        }
        None => tile("", "", 38.0).round().icon(Icon::User).muted(true),
    };
    list_item().capsule().padding(4.0).selected(g.active_sheet() == Some(&Sheet::Accounts)).on_click(|g: &mut Gui| g.toggle(Sheet::Accounts)).child(avatar).into()
}

/// Subtitle of an instance: its game version and loader.
pub fn describe(i: &Instance) -> String {
    let mc = i.mc_version.clone().unwrap_or_else(|| i.version_id.clone());
    if i.loaders.is_empty() {
        return format!("{} {mc}", tr!("loaders.vanilla"));
    }
    let mut parts = vec![mc];
    parts.extend(i.loaders.iter().map(|l| format!("{} {}", loader_name(l.kind), l.version)));
    parts.join(" · ")
}

fn instance_picker(g: &Gui) -> El<Gui> {
    let (title, sub, lead) = match g.m.selected_instance() {
        Some(i) => (i.name.clone(), describe(i), version_tile::<Gui>(instance_icon(&g.m, i), 34.0)),
        None => (tr!("instances.empty").to_string(), tr!("instances.empty_desc").to_string(), version_tile::<Gui>(VersionIcon::Grass, 34.0).muted(true)),
    };
    list_item()
        .capsule()
        .width(236.0)
        .padding(Insets::new(6.0, 10.0, 6.0, 6.0))
        .gap(10.0)
        .selected(matches!(g.active_sheet(), Some(Sheet::Instances | Sheet::NewInstance)))
        .on_click(|g: &mut Gui| {
            let s = if g.m.instances.ready().is_some_and(|l| l.is_empty()) { Sheet::NewInstance } else { Sheet::Instances };
            g.toggle(s);
        })
        .on_context_menu(|g: &mut Gui, at| {
            if let Some(id) = g.m.selected.clone() {
                g.open_instance_menu(&id, at);
            }
        })
        .child(lead)
        .child(column().grow(1.0).min_width(0.0).gap(1.0).child(label(title).truncate().weight(700).color(fg(0.95))).child(label(sub).truncate().caption()))
        .into()
}

fn launch(g: &Gui) -> El<Gui> {
    launch_control(g).width(156.0).height(46.0).into()
}

/// The launch button. Its label follows the launcher, instance and game state, for example
/// 'Preparing', 'Launch', 'Launching 42%' or 'Game running'. Clicking does the matching action.
/// The dock and the home page quick-launch widget share it. The caller sets its size.
pub fn launch_control(g: &Gui) -> LaunchButton<Gui> {
    let button = |text: &str, ic: Icon, state: LaunchState| launch_button::<Gui>(text, ic, state);
    let ready = g.m.status == Status::Ready;
    let Some(inst) = g.m.selected_instance().filter(|_| ready) else {
        return match (&g.m.status, g.m.instances.ready()) {
            (Status::Starting, _) | (Status::Ready, None) => button(tr!("common.loading"), Icon::Rocket, LaunchState::Busy(None)),
            (Status::Failed(_), _) => button(tr!("common.error"), Icon::Warning, LaunchState::Disabled).on_click(|g: &mut Gui| g.toggle_tasks()),
            (Status::Ready, Some(_)) => button(tr!("common.install"), Icon::Download, LaunchState::Ready).on_click(|g: &mut Gui| g.open(Sheet::NewInstance)),
        };
    };
    let id = inst.id.clone();
    if g.m.game(&id).is_some() {
        return button(tr!("dock.running"), Icon::Rocket, LaunchState::Running).on_click(|g: &mut Gui| g.toggle_tasks());
    }
    if let Some(t) = g.m.launch_task(&id) {
        let f = t.progress.fraction as f32;
        let state = if f > 0.0 { LaunchState::Busy(Some(f)) } else { LaunchState::Busy(None) };
        return button(&format!("{} {:.0}%", tr!("common.loading"), f * 100.0), Icon::Rocket, state).on_click(|g: &mut Gui| g.toggle_tasks());
    }
    button(tr!("dock.launch"), Icon::Rocket, LaunchState::Ready).on_click(move |g: &mut Gui| {
        if g.m.selected_account().is_none() {
            g.open(Sheet::Accounts);
        } else {
            g.bridge.launch(&mut g.m, &id);
        }
    })
}
