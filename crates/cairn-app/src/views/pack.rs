//! Imports a modpack: once it is recognized, the user confirms the name and the install folder.

use cairn_bridge::{FOLDER_CURRENT, InstallTarget, PackFormat, PackPreview, Remote};
use cairn_i18n::tr;
use cairn_icons::VersionIcon;
use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, button, column, icon, label, list_item, row, spinner, text_input};

use super::instances::folder_icon;

use super::{frame_auto, header};
use crate::gui::{Gui, PackForm, Sheet};
use crate::style::{RED, fg};
use crate::widgets::version_tile;

pub fn view(g: &Gui) -> El<Gui> {
    let lead = label(tr!("pack.drop_pack")).title();
    let body: El<Gui> = match &g.m.pack {
        Remote::Ready(p) => form(g, p),
        Remote::Failed(e) => column()
            .fill_width()
            .align(Align::Stretch)
            .gap(12.0)
            .padding(Insets::new(10.0, 22.0, 20.0, 22.0))
            .child(row().gap(10.0).child(icon(Icon::Warning).icon_size(20.0).color(RED)).child(label(tr!("pack.cannot_recognize")).weight(700)))
            .child(label(e.as_str()).caption())
            .child(row().justify(Justify::End).child(button(tr!("common.close")).on_click(|g: &mut Gui| g.close_pack())))
            .into(),
        _ => row().fill_width().justify(Justify::Center).gap(10.0).padding(34.0).child(spinner()).child(label(tr!("pack.identifying")).caption()).into(),
    };
    frame_auto(header(lead, Vec::new()), body)
}

fn format_name(f: PackFormat) -> &'static str {
    match f {
        PackFormat::Modrinth => tr!("pack.pack_modrinth"),
        PackFormat::CurseForge => tr!("pack.pack_curseforge"),
        PackFormat::MultiMc => tr!("pack.pack_multimc"),
    }
}

fn form(g: &Gui, p: &PackPreview) -> El<Gui> {
    let name = g.ui.pack.name.clone().unwrap_or_else(|| p.name.clone());
    let folder = g.ui.pack.folder.clone().unwrap_or_else(|| FOLDER_CURRENT.to_string());
    let loader = match &p.loader {
        Some(spec) => spec.name().to_string(),
        None => tr!("loaders.vanilla").to_string(),
    };
    let summary = column().grow(1.0).min_width(0.0).gap(2.0).child(label(&p.name).weight(700).color(fg(0.95)).truncate()).child(
        label(format!(
            "{} · Minecraft {} · {}{}",
            format_name(p.format),
            p.mc_version,
            loader,
            p.version.as_ref().map(|v| format!(" · v{v}")).unwrap_or_default()
        ))
        .caption()
        .truncate(),
    );

    let folders = g.m.folders();
    let places = folders.iter().map(|f| {
        let target = f.name.clone();
        list_item()
            .key(("pack-place", f.name.clone()))
            .padding(Insets::symmetric(10.0, 6.0))
            .gap(10.0)
            .selected(f.name == folder)
            .on_click(move |g: &mut Gui| g.ui.pack.folder = Some(target.clone()))
            .child(icon(folder_icon(f.kind)).icon_size(16.0))
            .child(label(f.label()).color(fg(0.9)).grow(1.0).truncate())
    });

    let problem = g.folder_name_problem(&folder, &name);
    let ok = !name.trim().is_empty() && problem.is_none();
    let col = column()
        .fill_width()
        .align(Align::Stretch)
        .gap(8.0)
        .padding(Insets::new(14.0, 20.0, 18.0, 20.0))
        .child(row().gap(12.0).child(version_tile::<Gui>(VersionIcon::Pack, 44.0)).child(summary))
        .child(label(tr!("pack.pack_instance_name")).secondary().margin(Insets::new(6.0, 0.0, 0.0, 0.0)))
        .child(text_input(&name).placeholder(tr!("pack.pack_instance_name")).on_change(|g: &mut Gui, s: String| g.ui.pack.name = Some(s)));
    let col = col.child(label(problem.unwrap_or_default()).caption().min_height(16.0).color(RED));
    col.child(label(tr!("pack.import_to")).secondary().margin(Insets::new(6.0, 0.0, 0.0, 0.0)))
        .child(column().fill_width().gap(2.0).align(Align::Stretch).children(places))
        .child(
            row()
                .fill_width()
                .gap(8.0)
                .justify(Justify::End)
                .padding(Insets::new(10.0, 0.0, 0.0, 0.0))
                .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.close_pack()))
                .child(button(tr!("pack.import_btn")).accent().icon(Icon::Download).enabled(ok).on_click(move |g: &mut Gui| {
                    let name = g.ui.pack.name.clone().or_else(|| g.m.pack.ready().map(|p| p.name.clone())).unwrap_or_default().trim().to_string();
                    let folder = g.ui.pack.folder.clone().unwrap_or_else(|| FOLDER_CURRENT.to_string());
                    let target = InstallTarget::Standard { root: folder, version_name: Some(name.clone()) };
                    g.bridge.import_pack(&mut g.m, &name, target);
                    // `m.pack` returns to Idle only once the bridge accepts the import.
                    // An invalid name keeps the dialog open.
                    if matches!(g.m.pack, Remote::Idle) {
                        g.ui.pack = PackForm::default();
                        g.ui.leaving = Some(Sheet::ImportPack);
                        g.ui.tasks_open = true;
                        g.ui.tasks_leaving = false;
                    }
                })),
        )
        .into()
}
