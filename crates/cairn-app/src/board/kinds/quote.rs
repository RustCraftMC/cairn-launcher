//! Daily quote: a short line and its source. The context menu can refresh it.
//! Quotes that are too long for a small cell get a smaller font automatically.

use cairn_i18n::tr;
use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, column, label, row};

use super::{Category, Ctx, Kind, Opt, Preset, card, span};
use crate::board::GAP;
use crate::board::glyph::{Glyph, glyph};
use crate::gui::Gui;
use crate::style::{accent, fg};

pub const KIND: Kind = Kind {
    id: "quote",
    name: "Quote",
    summary: "Curated quotes and verses",
    glyph: Glyph::Quote,
    category: Category::Info,
    min: span(3, 1),
    max: span(8, 3),
    presets: &[Preset { label: "Narrow", span: span(4, 1) }, Preset { label: "Medium", span: span(4, 2) }, Preset { label: "Large", span: span(6, 2) }],
    default: 1,
    render: view,
    options: &[Opt::Action { label: "Refresh Quote", icon: Icon::Refresh, run: refresh }],
};

fn refresh(g: &mut Gui) {
    g.bridge.refresh_hitokoto(&mut g.m);
}

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let (text, from) = match &g.m.hitokoto {
        Some(h) => (h.text.clone(), h.from.clone()),
        None => (tr!("board.quote_loading").to_string(), None),
    };
    let s = cx.span;
    if s.h == 1 {
        return card()
            .horizontal()
            .padding(Insets::symmetric(16.0, 8.0))
            .gap(10.0)
            .child(glyph(Glyph::Quote).size(18.0).color(accent().with_opacity(0.85)))
            .child(label(text).font_size(14.0).weight(600).color(fg(0.9)).truncate().grow(1.0).min_width(0.0))
            .into();
    }

    // Estimates how many characters fit from the cell's pixel size.
    // The font shrinks while the quote does not fit.
    let m = g.ui.board.metrics.get();
    let base: f32 = if s.w >= 6 || s.h >= 3 { 18.0 } else { 15.0 };
    let size = if m.ready() {
        let w = m.col_w() * f32::from(s.w) - GAP - 32.0;
        let h = m.row_h() * f32::from(s.h) - GAP - 32.0 - 30.0 - if from.is_some() { 18.0 } else { 0.0 };
        let chars = text.chars().count().max(1) as f32;
        let fits = |f: f32| (w / f).floor() * (h / (f * 1.45)).floor();
        let mut f = base;
        while f > 11.5 && fits(f) < chars {
            f -= 0.5;
        }
        f
    } else {
        base
    };

    let mut c = card()
        .padding(Insets::new(14.0, 16.0, 12.0, 16.0))
        .gap(6.0)
        .child(glyph(Glyph::Quote).size(24.0).color(accent().with_opacity(0.85)))
        .child(label(text).font_size(size).weight(600).color(fg(0.9)))
        .child(column().grow(1.0));
    if let Some(from) = from {
        c = c.child(row().fill_width().justify(Justify::End).align(Align::Center).child(label(format!("{from}")).font_size(12.0).color(fg(0.5)).truncate()));
    }
    c.into()
}
