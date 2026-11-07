//! Quick launch: the current instance (icon, name and version) and its launch button.
//! When the widget is tall enough, it also lists other recently played instances to switch to.

use cairn_bridge::local_now;
use cairn_i18n::tr;
use cairn_icons::VersionIcon;
use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, button, column, label, list_item, row};

use super::{Category, Ctx, Kind, Preset, card, heading, instance_label, recent_instances, relative_day, span, today};
use crate::board::glyph::Glyph;
use crate::gui::{Gui, Sheet};
use crate::style::fg;
use crate::views::{describe, instance_icon, launch_control};
use crate::widgets::{LaunchState, launch_button, version_tile};

pub const KIND: Kind = Kind {
    id: "launch",
    name: "Launch",
    summary: "Current instance and quick launch",
    glyph: Glyph::Icon(Icon::Rocket),
    category: Category::Game,
    min: span(2, 2),
    max: span(6, 5),
    presets: &[Preset { label: "Small", span: span(2, 2) }, Preset { label: "Medium", span: span(4, 2) }, Preset { label: "Large", span: span(4, 4) }],
    default: 1,
    render: view,
    options: &[],
};

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let s = cx.span;
    let inst = g.m.selected_instance();
    let (name, detail, icon, muted) = match inst {
        Some(i) => (i.name.clone(), describe(i), instance_icon(&g.m, i), false),
        None => (tr!("board.launch_no_instance").to_string(), tr!("board.launch_install_hint").to_string(), VersionIcon::Grass, true),
    };
    let control = |height: f32| -> El<Gui> {
        if cx.preview {
            launch_button::<Gui>(tr!("board.launch_btn"), Icon::Rocket, LaunchState::Ready).height(height).fill_width().into()
        } else {
            launch_control(g).height(height).fill_width().into()
        }
    };
    let open_list = |g: &mut Gui| {
        let s = if g.m.instances.ready().is_some_and(|l| l.is_empty()) { Sheet::NewInstance } else { Sheet::Instances };
        g.toggle(s);
    };

    if s.w <= 3 {
        return card()
            .align(Align::Center)
            .gap(6.0)
            .child(
                column()
                    .grow(1.0)
                    .justify(Justify::Center)
                    .align(Align::Center)
                    .gap(6.0)
                    .child(version_tile::<Gui>(icon, 52.0).muted(muted))
                    .child(label(name).weight(700).color(fg(0.95)).truncate()),
            )
            .child(control(36.0))
            .into();
    }

    let mut c = card().gap(10.0).child(
        list_item()
            .fill_width()
            .padding(Insets::new(4.0, 6.0, 4.0, 4.0))
            .gap(12.0)
            .on_click(open_list)
            .child(version_tile::<Gui>(icon, if s.h >= 3 { 60.0 } else { 52.0 }).muted(muted))
            .child(
                column()
                    .grow(1.0)
                    .min_width(0.0)
                    .gap(3.0)
                    .child(label(name).font_size(17.0).weight(700).color(fg(0.96)).truncate())
                    .child(label(detail).font_size(12.5).color(fg(0.55)).truncate()),
            )
            .child(cairn_ui::icon(Icon::ChevronRight).icon_size(14.0).color(fg(0.35))),
    );

    let rows = (s.h as usize).saturating_sub(2) * 2;
    if rows > 0 {
        let today = today(&local_now());
        let current = g.m.selected.clone();
        let recent: Vec<(String, i64)> =
            recent_instances(&g.m.playtime).into_iter().filter(|(id, _)| Some(id) != current.as_ref() && g.m.instance(id).is_some()).take(rows).collect();
        c = c.child(heading(Glyph::Clock, tr!("board.launch_recent_played")));
        if recent.is_empty() {
            c = c.child(label(tr!("board.launch_other_instances_hint")).caption());
        }
        let mut list = column().fill_width().gap(2.0);
        for (id, day) in recent {
            let icon = g.m.instance(&id).map(|i| instance_icon(&g.m, i)).unwrap_or(VersionIcon::Grass);
            let target = id.clone();
            list = list.child(
                list_item()
                    .fill_width()
                    .padding(Insets::new(4.0, 8.0, 4.0, 4.0))
                    .gap(10.0)
                    .on_click(move |g: &mut Gui| g.bridge.select_instance(&mut g.m, &target))
                    .child(version_tile::<Gui>(icon, 26.0))
                    .child(label(instance_label(g, &id)).weight(600).color(fg(0.88)).truncate().grow(1.0).min_width(0.0))
                    .child(label(relative_day(day, today)).caption()),
            );
        }
        c = c.child(list);
    }

    c = c.child(column().grow(1.0));
    if inst.is_none() && !cx.preview {
        return c.child(button(tr!("library.install_game")).icon(Icon::Download).accent().on_click(|g: &mut Gui| g.open(Sheet::NewInstance))).into();
    }
    c.child(row().fill_width().child(control(40.0))).into()
}
