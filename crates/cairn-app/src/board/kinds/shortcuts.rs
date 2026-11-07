//! Shortcuts: one-click access to common panels, including custom ones.
//! Larger cells fit more entries.

use cairn_i18n::tr;
use std::rc::Rc;

use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, column, icon, label, list_item, row};

use super::{Category, Ctx, Kind, Preset, card, span};
use crate::board::glyph::Glyph;
use crate::gui::{Gui, Sheet};
use crate::style::{accent, fg};

pub const KIND: Kind = Kind {
    id: "shortcuts",
    name: "Shortcuts",
    summary: "Quick shortcuts to frequent and custom panels",
    glyph: Glyph::Bolt,
    category: Category::Game,
    min: span(2, 1),
    max: span(6, 3),
    presets: &[Preset { label: "Small", span: span(2, 2) }, Preset { label: "Wide", span: span(4, 1) }, Preset { label: "Medium", span: span(4, 2) }],
    default: 0,
    render: view,
    options: &[],
};

type Action = Rc<dyn Fn(&mut Gui)>;

fn items(g: &Gui) -> Vec<(String, Icon, Action)> {
    let mut out: Vec<(String, Icon, Action)> = vec![
        (tr!("board.shortcuts_new_instance").into(), Icon::Plus, Rc::new(|g: &mut Gui| g.open(Sheet::NewInstance))),
        (tr!("board.shortcuts_versions").into(), Icon::Cube, Rc::new(|g: &mut Gui| g.toggle(Sheet::Instances))),
        (tr!("board.shortcuts_library").into(), Icon::Package, Rc::new(|g: &mut Gui| g.toggle(Sheet::Library))),
        (tr!("accounts.title").into(), Icon::User, Rc::new(|g: &mut Gui| g.toggle(Sheet::Accounts))),
        (tr!("board.shortcuts_settings").into(), Icon::Gear, Rc::new(|g: &mut Gui| g.toggle(Sheet::Settings))),
        (
            tr!("instances.game_dir").into(),
            Icon::Folder,
            Rc::new(|g: &mut Gui| {
                if let Some(dir) = g.m.selected_instance().map(|i| i.game_dir.clone()) {
                    g.bridge.open_path(&dir);
                }
            }),
        ),
    ];
    for p in &g.m.panels {
        let id = p.id.clone();
        out.push((p.title.clone(), Icon::Globe, Rc::new(move |g: &mut Gui| g.toggle(Sheet::CustomPanel(id.clone())))));
    }
    out
}

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let s = cx.span;
    let cols = usize::from(s.w.max(1));
    let rows = usize::from(s.h.max(1));
    let with_label = s.h >= 2;
    let all = items(g);
    let a = accent();
    let mut grid = column().fill_width().grow(1.0).gap(4.0).justify(Justify::SpaceEvenly);
    for chunk in all.chunks(cols).take(rows) {
        let mut line = row().fill_width().gap(4.0);
        for (name, ic, action) in chunk {
            let action = action.clone();
            let badge =
                row().size(34.0, 34.0).justify(Justify::Center).background(a.with_opacity(0.15)).corner_radius(11.0).child(icon(*ic).icon_size(17.0).color(a));
            let mut tile = column().fill_width().align(Align::Center).gap(4.0).child(badge);
            if with_label {
                tile = tile.child(label(name).font_size(11.5).weight(600).color(fg(0.78)).truncate());
            }
            let mut item = list_item().width(0.0).grow(1.0).padding(Insets::symmetric(2.0, 5.0)).child(tile);
            if !cx.preview {
                item = item.on_click(move |g: &mut Gui| action(g));
            }
            line = line.child(item);
        }
        // Pad a short last row with spacers, so every item keeps the same width.
        for _ in chunk.len()..cols {
            line = line.child(row().width(0.0).grow(1.0));
        }
        grid = grid.child(line);
    }
    card().padding(Insets::all(if s.h == 1 { 8.0 } else { 10.0 })).child(grid).into()
}
