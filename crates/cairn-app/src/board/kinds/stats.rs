//! Playtime stats: this week's hours and a 7-day bar chart.
//! When there is room, it adds today, the streak, the total and the most played instance.

use cairn_bridge::local_now;
use cairn_i18n::tr;
use cairn_ui::{
    Align, Change, El, FontFamily, Insets, Justify, LayoutCx, MeasureInput, PaintCx, Painter, Point, Rect, Size, Style, Styled, TextLayout, TextStyle, Widget,
    column, label, row,
};

use super::{Category, Ctx, Daily, Kind, Preset, card, figure, heading, hours, instance_label, most_played, span, weekday, weekday_label};
use crate::board::glyph::{Glyph, glyph};
use crate::gui::Gui;
use crate::style::{accent, fg};

pub const KIND: Kind = Kind {
    id: "stats",
    name: "Stats",
    summary: "Weekly playtime, 7-day trend and cumulative statistics",
    glyph: Glyph::Chart,
    category: Category::Stats,
    min: span(2, 2),
    max: span(6, 4),
    presets: &[Preset { label: "Small", span: span(2, 2) }, Preset { label: "Medium", span: span(4, 2) }, Preset { label: "Large", span: span(4, 3) }],
    default: 0,
    render: view,
    options: &[],
};

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let now = local_now();
    let mut daily = Daily::from_store(&g.m.playtime, &now, 53);
    if cx.preview && daily.is_empty() {
        daily = Daily::sample(&now, 53);
    }
    let today = daily.today;
    let wd = weekday(today) as i64;
    let week = daily.sum(today - wd, today);
    let last = daily.sum(today - wd - 7, today - 7);
    let compare = match week - last {
        _ if week == 0.0 && last == 0.0 => tr!("board.stats_empty_week").to_string(),
        d if d.abs() < 0.05 => tr!("board.stats_same_as_last_week").to_string(),
        d if d > 0.0 => tr!("board.stats_more_than_last_week", hours = hours(d)),
        d => tr!("board.stats_less_than_last_week", hours = hours(-d)),
    };
    let bars: Vec<(f32, bool, usize)> = (0..7)
        .map(|i| {
            let d = today - 6 + i;
            (daily.day(d), d == today, weekday(d))
        })
        .collect();
    let s = cx.span;

    let hero = column()
        .grow(1.0)
        .fill_height()
        .min_width(0.0)
        .gap(2.0)
        .child(heading(Glyph::Chart, tr!("board.stats_week_play")))
        .child(figure(hours(week), "h", if s.w >= 4 { 34.0 } else { 30.0 }))
        .child(label(compare).font_size(11.5).color(fg(0.5)).truncate())
        .child(column().grow(1.0).min_height(6.0))
        .child(BarSpark::new(bars).fill_width().height(if s.h >= 3 { 70.0 } else { 46.0 }));

    if s.w < 4 {
        return card().padding(Insets::all(13.0)).child(hero).into();
    }

    // The side column is narrow: a name beside its value wraps per letter and clips the value.
    // Each item therefore takes two lines, name above and value below.
    let metric = |g: Glyph, name: String, value: String| -> El<Gui> {
        row()
            .fill_width()
            .gap(9.0)
            .child(glyph(g).size(14.0).color(fg(0.45)))
            .child(
                column()
                    .grow(1.0)
                    .min_width(0.0)
                    .gap(1.0)
                    .align(Align::Stretch)
                    .child(label(name).font_size(11.5).color(fg(0.5)).truncate())
                    .child(label(value).font_size(13.5).weight(700).color(fg(0.92)).truncate()),
            )
            .into()
    };
    let mut metrics = vec![
        (Glyph::Clock, tr!("board.stats_today").to_string(), format!("{} h", hours(daily.day(today)))),
        (Glyph::Flame, tr!("board.stats_streak").to_string(), tr!("board.streak_days", days = daily.streak())),
        (Glyph::Calendar, tr!("board.stats_total").to_string(), format!("{} h", hours(daily.total))),
    ];
    if s.h >= 3 {
        let top = if cx.preview && g.m.playtime.records.is_empty() {
            Some(("1.20.1".to_string(), 128.0))
        } else {
            most_played(&g.m.playtime).map(|(id, h)| (instance_label(g, &id), h))
        };
        if let Some((name, h)) = top {
            // Hours go after the label; the instance name (often long) gets the value line
            // on its own.
            metrics.push((Glyph::Icon(cairn_ui::Icon::Gamepad), format!("{} · {} h", tr!("board.stats_most_played"), hours(h)), name));
        }
    }
    // Show as many items as the height fits and drop the rest from the end,
    // so a short window does not overflow the card.
    let room = g.ui.board.metrics.get().tile_size(s).height - 28.0;
    let fit = (((room + METRIC_GAP) / (METRIC_HEIGHT + METRIC_GAP)).floor() as usize).max(1);
    let side = column()
        .width(0.0)
        .grow(0.9)
        .fill_height()
        .gap(METRIC_GAP)
        .justify(Justify::Center)
        .children(metrics.into_iter().take(fit).map(|(icon, name, value)| metric(icon, name, value)));
    // Both halves share the width by ratio, not by content width, so a wide hero
    // cannot squeeze the side column down to a sliver.
    card().horizontal().align(Align::Stretch).gap(18.0).child(hero.width(0.0)).child(side).into()
}

/// Side-column item height (one name line and one value line) and the gap between items.
const METRIC_HEIGHT: f32 = 38.0;
const METRIC_GAP: f32 = 8.0;

/// Bar chart of the last 7 days: today uses the accent color, the rest are faded.
/// Each bar sits on a faint track.
struct BarSpark {
    style: Style,
    key: Option<u64>,
    /// (hours, is today, weekday index) per bar.
    bars: Vec<(f32, bool, usize)>,
    labels: Vec<TextLayout>,
    content: Rect,
}

impl BarSpark {
    fn new(bars: Vec<(f32, bool, usize)>) -> Self {
        BarSpark { style: Style::default(), key: None, bars, labels: Vec::new(), content: Rect::ZERO }
    }

    fn labelled(&self) -> bool {
        self.content.height >= 40.0
    }
}

into_el!(BarSpark);

impl<S: 'static> Widget<S> for BarSpark {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        if new.bars == self.bars {
            return Change::NONE;
        }
        let relabel = new.bars.iter().map(|b| b.2).ne(self.bars.iter().map(|b| b.2));
        self.bars = new.bars;
        if relabel {
            self.labels.clear();
            Change::LAYOUT
        } else {
            Change::PAINT
        }
    }

    fn measure(&mut self, _cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        Size::new(input.known_width.unwrap_or(0.0), input.known_height.unwrap_or(0.0))
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        self.content = content;
        if self.labels.len() != self.bars.len() {
            let style = TextStyle { font_size: 10.0, line_height: 1.2, weight: 600, family: FontFamily::SansSerif };
            self.labels = self.bars.iter().map(|b| TextLayout::new(cx.text(), weekday_label(b.2 % 7), &style)).collect();
        }
    }

    fn paint(&mut self, _cx: &mut PaintCx, p: &mut dyn Painter) {
        let n = self.bars.len();
        if n == 0 {
            return;
        }
        let c = self.content;
        let label_h = if self.labelled() { 14.0 } else { 0.0 };
        let area = Rect::new(c.x, c.y, c.width, (c.height - label_h).max(4.0));
        let max = self.bars.iter().map(|b| b.0).fold(1.0f32, f32::max);
        let slot = area.width / n as f32;
        let bar_w = (slot * 0.52).clamp(4.0, 16.0);
        let a = accent();
        for (i, (h, today, _)) in self.bars.iter().enumerate() {
            let x = area.x + slot * i as f32 + (slot - bar_w) / 2.0;
            let track = Rect::new(x, area.y, bar_w, area.height);
            p.fill_rounded_rect(track, bar_w / 2.0, fg(0.06));
            if *h > 0.0 {
                let bh = (area.height * (h / max)).max(bar_w.min(area.height));
                let bar = Rect::new(x, area.bottom() - bh, bar_w, bh);
                p.fill_rounded_rect(bar, bar_w / 2.0, if *today { a } else { a.with_opacity(0.45) });
            }
            if let Some(l) = self.labels.get(i).filter(|_| label_h > 0.0) {
                let s = l.size();
                p.draw_text(l, Point::new(x + bar_w / 2.0 - s.width / 2.0, c.bottom() - s.height), if *today { a } else { fg(0.38) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use cairn_bridge::{Appearance, Bridge, Sink};
    use cairn_i18n::Language;
    use cairn_ui::Rect;
    use cairn_ui::testing::Headless;

    use super::*;
    use crate::board::Metrics;

    /// Renders the 4x3 stats widget with sample data in a window, for a grid row `row_h` tall.
    fn show(rt: &tokio::runtime::Runtime, row_h: f32) -> Headless<Gui> {
        let sink: Sink = Arc::new(|_| true);
        let g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        let grid = Rect::new(0.0, 0.0, 1200.0, row_h * 6.0);
        g.ui.board.metrics.set(Metrics { grid, canvas: grid });
        let size = g.ui.board.metrics.get().tile_size(span(4, 3));
        let view = move |g: &Gui| {
            let options = BTreeMap::new();
            let cx = Ctx { kind: &KIND, span: span(4, 3), options: &options, preview: true };
            column().width(size.width).height(size.height).child(view(g, &cx))
        };
        let theme = crate::style::theme(&Appearance::default(), None, false);
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut app = Headless::with_theme(g, view, size.width, size.height, 1.0, theme, text);
        app.render();
        app
    }

    #[test]
    fn side_metrics_take_two_lines_and_drop_what_does_not_fit() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(Language::ZhCn, || {
            let app = show(&rt, 90.0);
            let most_played = format!("{} · 128 h", tr!("board.stats_most_played"));
            for name in [tr!("board.stats_today"), tr!("board.stats_streak"), tr!("board.stats_total"), most_played.as_str()] {
                let b = app.find(name).unwrap_or_else(|| panic!("缺 {name}")).bounds;
                assert!(b.height < 22.0 && b.width > 24.0, "{name}: {b:?} 应该是一行");
            }
            assert!(app.find("1.20.1").is_some(), "最常玩的实例名在数值那一行");

            // At a 50-high row the four items do not fit: the last ones are dropped
            // instead of overflowing the card.
            let app = show(&rt, 50.0);
            assert!(app.find(tr!("board.stats_today")).is_some());
            assert!(app.find(tr!("board.stats_total")).is_none() && app.find(&most_played).is_none());
        });
    }
}
