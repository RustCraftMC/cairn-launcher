//! Greeting widget: player avatar, time-of-day salutation and date.
//! When there is room, it also shows a hitokoto quote and an account switch button.

use cairn_bridge::{AccountKind, local_now};
use cairn_i18n::tr;
use cairn_ui::{Align, El, Insets, Justify, Styled, button, column, label, row, tag};

use super::{Category, Ctx, Kind, Preset, card, span, weekday_label};
use crate::board::glyph::Glyph;
use crate::gui::{Gui, Sheet};
use crate::style::fg;
use crate::widgets::{initial, tile};

pub const KIND: Kind = Kind {
    id: "greeting",
    name: "Greeting",
    summary: "Player avatar, greeting and daily quote",
    glyph: Glyph::Sparkle,
    category: Category::Info,
    min: span(2, 1),
    max: span(8, 4),
    presets: &[Preset { label: "Small", span: span(2, 2) }, Preset { label: "Medium", span: span(4, 2) }, Preset { label: "Large", span: span(4, 3) }],
    default: 1,
    render: view,
    options: &[],
};

pub fn salutation(hour: u8) -> &'static str {
    match hour {
        5..=10 => tr!("board.greeting_morning"),
        11..=12 => tr!("board.greeting_noon"),
        13..=17 => tr!("board.greeting_afternoon"),
        18..=22 => tr!("board.greeting_evening"),
        _ => tr!("board.greeting_night"),
    }
}

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let t = local_now();
    let account = g.m.selected_account();
    let name = account.map(|a| a.name.clone()).unwrap_or_else(|| tr!("custom_panel.default_player").into());
    let hello = salutation(t.hour);
    let date = if cairn_i18n::is_chinese() {
        format!("{}月{}日 星期{}", t.month, t.day, weekday_label(t.weekday as usize % 7))
    } else {
        format!("{}, {}/{}", weekday_label(t.weekday as usize % 7), t.month, t.day)
    };
    let avatar = |side: f32| {
        let mut a = match account {
            Some(a) => tile(&a.name, initial(&a.name), side).round(),
            None => tile("", "", side).round().icon(cairn_ui::Icon::User).muted(true),
        };
        if let Some(img) = account.and_then(|a| g.icon(&format!("avatar:{}", a.id))) {
            a = a.image(img);
        }
        a
    };
    let Ctx { span: s, .. } = *cx;

    if s.h == 1 {
        return card()
            .horizontal()
            .padding(Insets::symmetric(14.0, 8.0))
            .gap(10.0)
            .child(avatar(34.0))
            .child(
                column()
                    .grow(1.0)
                    .min_width(0.0)
                    .gap(1.0)
                    .child(label(format!("{hello},{name}")).weight(700).color(fg(0.95)).truncate())
                    .child(label(date).caption().truncate()),
            )
            .into();
    }

    if s.w <= 3 {
        return card()
            .align(Align::Center)
            .justify(Justify::Center)
            .gap(6.0)
            .child(avatar(if s.h >= 3 { 64.0 } else { 46.0 }))
            .child(label(hello).font_size(12.5).weight(600).color(fg(0.55)))
            .child(label(name).weight(700).color(fg(0.95)).truncate())
            .into();
    }

    let quote = g.m.hitokoto.as_ref();
    let mut c = card().gap(10.0).child(
        row().fill_width().gap(12.0).child(avatar(if s.h >= 3 { 58.0 } else { 48.0 })).child(
            column()
                .grow(1.0)
                .min_width(0.0)
                .gap(2.0)
                .child(label(format!("{hello},{name}")).font_size(19.0).weight(700).color(fg(0.96)).truncate())
                .child(label(date).font_size(12.5).color(fg(0.55)).truncate()),
        ),
    );
    c = c.child(column().grow(1.0));
    if let Some(q) = quote {
        let text = format!("“{}”", q.text);
        let mut l = label(text).font_size(13.0).color(fg(0.68));
        if s.h <= 2 {
            l = l.truncate();
        }
        c = c.child(l);
        if s.h >= 3
            && let Some(from) = &q.from
        {
            c = c.child(row().fill_width().justify(Justify::End).child(label(format!("{from}")).caption().truncate()));
        }
    }
    if s.h >= 3 && !cx.preview {
        let kind = match account.map(|a| &a.kind) {
            Some(AccountKind::Microsoft) => tr!("accounts.microsoft"),
            Some(_) => tr!("accounts.offline"),
            None => tr!("accounts.empty"),
        };
        c = c.child(
            row()
                .fill_width()
                .gap(8.0)
                .child(tag(kind))
                .child(row().grow(1.0))
                .child(button(tr!("accounts.switch")).on_click(|g: &mut Gui| g.toggle(Sheet::Accounts))),
        );
    }
    c.into()
}
