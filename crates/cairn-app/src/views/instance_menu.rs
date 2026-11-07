//! Right-click menu for an instance, shared by the dock's current instance and the instance rows.

use cairn_bridge::{Instance, Status};
use cairn_i18n::tr;
use cairn_ui::{El, Icon, Insets, Justify, Styled, anchored, button, column, divider, label, menu, menu_item, reveal, row};

use crate::gui::{Gui, InstanceMenu};
use crate::style::fg;

/// While `leaving` is true, the menu is fading out and is removed once the fade ends.
pub fn view(g: &Gui, m: &InstanceMenu, leaving: bool) -> El<Gui> {
    let Some(inst) = g.m.instance(&m.id) else {
        return row().into();
    };
    let body = if m.confirm { confirm(inst) } else { items(g, &inst.id) };
    let body = reveal()
        .from_above()
        .offset(6.0)
        .duration(std::time::Duration::from_millis(160))
        .exit_duration(std::time::Duration::from_millis(120))
        .key(("instance-menu", m.id.as_str(), m.confirm))
        .leaving(leaving)
        .on_left(|g: &mut Gui| g.ui.instance_menu_leaving = None)
        .child(body);
    let mut a = anchored(m.at);
    if !leaving {
        a = a.on_dismiss(|g: &mut Gui| g.close_instance_menu());
    }
    a.child(body).into()
}

fn items(g: &Gui, id: &str) -> El<Gui> {
    let busy = g.m.status != Status::Ready || g.m.game(id).is_some() || g.m.launch_task(id).is_some();
    let open = |name: &str, sub: &'static str, ic: Icon| {
        let id = id.to_string();
        menu_item(name).icon(ic).on_click(move |g: &mut Gui| {
            g.close_instance_menu();
            open_dir(g, &id, sub);
        })
    };
    let (launch, settings) = (id.to_string(), id.to_string());
    menu()
        .min_width(200.0)
        .child(menu_item(tr!("dock.launch")).icon(Icon::Play).enabled(!busy).on_click(move |g: &mut Gui| {
            g.close_instance_menu();
            g.launch_instance(&launch);
        }))
        .child(open(tr!("instance_menu.open_game_dir"), "", Icon::Folder))
        .child(open(tr!("instance_menu.open_mods_dir"), "mods", Icon::Package))
        .child(open(tr!("instance_menu.open_saves_dir"), "saves", Icon::Globe))
        .child(open(tr!("instance_menu.open_screenshots_dir"), "screenshots", Icon::Camera))
        .child(menu_item(tr!("instance_menu.instance_settings")).icon(Icon::Gear).on_click(move |g: &mut Gui| g.open_version(&settings)))
        .child(divider().fill_width().margin(Insets::symmetric(8.0, 4.0)))
        .child(menu_item(tr!("instance_menu.delete_instance")).icon(Icon::Trash).danger().on_click(|g: &mut Gui| {
            if let Some(m) = &mut g.ui.instance_menu {
                m.confirm = true;
            }
        }))
        .into()
}

/// What deleting an instance removes: only its version folder.
/// With version isolation, saves and mods are inside it.
/// With a shared game folder they are outside and unaffected.
/// Deleting does not go through the recycle bin.
pub fn delete_scope(inst: &Instance) -> &'static str {
    if inst.game_dir == inst.root { tr!("instance_menu.delete_shared_tip") } else { tr!("instance_menu.delete_isolated_tip") }
}

fn confirm(inst: &Instance) -> El<Gui> {
    let del = inst.id.clone();
    let body = column()
        .gap(10.0)
        .padding(Insets::new(6.0, 8.0, 6.0, 8.0))
        .max_width(320.0)
        .child(label(tr!("instance_menu.delete_confirm_title", name = &inst.name)).weight(700).color(fg(0.95)).truncate())
        .child(label(delete_scope(inst)).caption())
        .child(row().fill_width().justify(Justify::End).gap(8.0).child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.close_instance_menu())).child(
            button(tr!("common.delete")).icon(Icon::Trash).on_click(move |g: &mut Gui| {
                g.close_instance_menu();
                // With the instance list open, the row collapses before it is deleted. From the dock the list is
                // not on screen, so the delete is immediate.
                if g.ui.sheet == Some(crate::gui::Sheet::Instances) {
                    g.remove_after_collapse(crate::gui::Removal::Instance(del.clone()));
                } else {
                    g.bridge.delete_instance(&mut g.m, &del);
                }
            }),
        ));
    menu().width(260.0).child(body).into()
}

/// Opens a subfolder of the game folder. A missing folder is created first, because a vanilla
/// instance has no mods folder and Explorer would open a different place otherwise.
pub fn open_dir(g: &mut Gui, id: &str, sub: &str) {
    let Some(inst) = g.m.instance(id) else { return };
    let dir = if sub.is_empty() { inst.game_dir.clone() } else { inst.game_dir.join(sub) };
    let _ = std::fs::create_dir_all(&dir);
    g.bridge.open_path(&dir);
}
