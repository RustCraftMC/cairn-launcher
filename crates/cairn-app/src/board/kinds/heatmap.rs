//! Playtime heatmap in the style of the GitHub contribution graph: one cell per day.
//! The whole graph is one custom-drawn widget, not one widget per cell. Width sets how many weeks
//! show (at most 53); when space is short, only the latest weeks show, so cells stay large.

use cairn_bridge::local_now;
use cairn_i18n::tr;
use cairn_ui::{
    Change, Color, El, Event, EventCx, FontFamily, Insets, LayoutCx, MeasureInput, PaintCx, Painter, Point, Rect, Shadow, Size, Style, Styled, TextLayout,
    TextStyle, TextSystem, Widget, label, row,
};

use super::{Category, Ctx, Daily, Kind, Opt, Preset, card, civil_from_days, heading, hours, span, weekday, weekday_label};
use crate::board::glyph::{Glyph, glyph};
use crate::gui::Gui;
use crate::style::{accent, current, fg};

pub const KIND: Kind = Kind {
    id: "heatmap",
    name: "Heatmap",
    summary: "One cell per day, darker means longer playtime",
    glyph: Glyph::Calendar,
    category: Category::Stats,
    min: span(4, 2),
    max: span(12, 4),
    presets: &[Preset { label: "Medium", span: span(6, 2) }, Preset { label: "Wide", span: span(12, 2) }, Preset { label: "Large", span: span(8, 3) }],
    default: 2,
    render: view,
    options: &[Opt::Choice { key: "palette", label: "Palette", choices: &[("accent", "Accent"), ("green", "Green")], default: "accent" }],
};

/// A year spans at most 53 week columns.
const WEEKS: usize = 53;

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let now = local_now();
    let mut daily = Daily::from_store(&g.m.playtime, &now, WEEKS);
    if cx.preview && daily.is_empty() {
        daily = Daily::sample(&now, WEEKS);
    }
    let s = cx.span;
    let streak = daily.streak();
    let summary = if daily.is_empty() {
        tr!("board.heatmap_empty").to_string()
    } else {
        tr!("board.heatmap_summary", hours = hours(daily.hours.iter().sum()), days = daily.active_days())
    };
    let mut head = row().fill_width().gap(10.0).child(heading(Glyph::Calendar, tr!("board.heatmap_history")));
    if s.w >= 5 {
        head = head.child(label(summary).font_size(12.0).color(fg(0.45)).truncate().min_width(0.0).shrink(1.0));
    }
    head = head.child(row().grow(1.0));
    if streak > 0 && s.w >= 4 {
        head = head.child(streak_badge(streak));
    }
    card().gap(8.0).child(head).child(HeatmapView::new(daily, palette(cx.opt("palette")), !cx.preview).fill_width().grow(1.0).min_height(0.0)).into()
}

fn streak_badge(days: u32) -> El<Gui> {
    let a = accent();
    let text = if current().light { a.mix(Color::BLACK, 0.35) } else { a.mix(Color::WHITE, 0.25) };
    row()
        .gap(4.0)
        .padding(Insets::symmetric(9.0, 3.0))
        .background(a.with_opacity(0.16))
        .corner_radius(11.0)
        .child(glyph(Glyph::Flame).size(13.0).color(a))
        .child(label(tr!("board.streak_days", days = days)).font_size(12.0).weight(700).color(text))
        .into()
}

/// Five levels of color: no play, under 30 minutes, under 1.5 hours,
/// under 3 hours, and 3 hours or more.
fn palette(choice: &str) -> [Color; 5] {
    let light = current().light;
    let empty = fg(if light { 0.07 } else { 0.075 });
    match choice {
        "green" if light => [empty, Color::hex(0x9BE9A8), Color::hex(0x40C463), Color::hex(0x30A14E), Color::hex(0x216E39)],
        "green" => [empty, Color::hex(0x0E4429), Color::hex(0x006D32), Color::hex(0x26A641), Color::hex(0x39D353)],
        _ => {
            let a = accent();
            [empty, a.with_opacity(0.28), a.with_opacity(0.5), a.with_opacity(0.75), a]
        }
    }
}

fn level(h: f32) -> usize {
    match h {
        h if h < 1.0 / 60.0 => 0,
        h if h < 0.5 => 1,
        h if h < 1.5 => 2,
        h if h < 3.0 => 3,
        _ => 4,
    }
}

/// Laid-out geometry: shown weeks, cell size and origin, relative to the content's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geo {
    weeks: usize,
    /// Day of the top-left cell, a Monday.
    first: i64,
    pitch: f32,
    cell: f32,
    origin: Point,
    labels_right: f32,
    /// Vertical center of the legend; `None` when there is no room for it.
    legend_y: Option<f32>,
}

struct HeatmapView {
    style: Style,
    key: Option<u64>,
    daily: Daily,
    palette: [Color; 5],
    /// Previews ignore the mouse.
    live: bool,
    content: Rect,
    geo: Option<Geo>,
    /// Inputs the geometry was computed from: size, start day and day count.
    geo_key: (f32, f32, i64, usize),
    months: Vec<(f32, TextLayout)>,
    weekdays: Vec<(f32, TextLayout)>,
    legend: Option<(TextLayout, TextLayout)>,
    hover: Option<i64>,
    tip: Option<TextLayout>,
}

impl HeatmapView {
    fn new(daily: Daily, palette: [Color; 5], live: bool) -> Self {
        HeatmapView {
            style: Style::default(),
            key: None,
            daily,
            palette,
            live,
            content: Rect::ZERO,
            geo: None,
            geo_key: (0.0, 0.0, 0, 0),
            months: Vec::new(),
            weekdays: Vec::new(),
            legend: None,
            hover: None,
            tip: None,
        }
    }

    fn total_weeks(&self) -> usize {
        self.daily.hours.len().div_ceil(7)
    }

    /// Coordinates are relative to the top-left corner of the content area.
    fn layout(&mut self, ts: &mut TextSystem) {
        let c = Rect::new(0.0, 0.0, self.content.width, self.content.height);
        let small = |size: f32| TextStyle { font_size: size, line_height: 1.2, weight: 600, family: FontFamily::SansSerif };
        let labels = c.width >= 240.0;
        let label_w = if labels { 20.0 } else { 0.0 };
        let month_h = if c.height >= 92.0 { 16.0 } else { 0.0 };
        let legend_h = if c.height >= 126.0 && c.width >= 300.0 { 20.0 } else { 0.0 };
        let grid_h = (c.height - month_h - legend_h).max(14.0);
        let pitch_h = (grid_h / 7.0).min(22.0);
        let avail = (c.width - label_w).max(10.0);
        let total = self.total_weeks().max(1);
        let fit = (avail / pitch_h).floor().max(1.0) as usize;
        let weeks = fit.min(total);
        // If the whole year fits, cells use the tighter of width and height.
        // If only some weeks fit, they follow the height and stop shrinking.
        let pitch = if fit >= total { pitch_h.min(avail / total as f32) } else { pitch_h };
        let gap = (pitch * 0.2).clamp(1.5, 4.0);
        let cell = (pitch - gap).max(2.0);
        let grid_w = weeks as f32 * pitch - gap;
        let x0 = c.x + (c.width - label_w - grid_w) / 2.0;
        let origin = Point::new(x0 + label_w, c.y + month_h + (grid_h - (7.0 * pitch - gap)) / 2.0);
        let first = self.daily.start + (total - weeks) as i64 * 7;
        let geo = Geo { weeks, first, pitch, cell, origin, labels_right: origin.x - 5.0, legend_y: (legend_h > 0.0).then(|| c.bottom() - legend_h / 2.0) };

        // Month labels go on the column whose Monday starts a new month,
        // skipping ones that do not fit.
        self.months.clear();
        if month_h > 0.0 {
            let mut last_right = f32::MIN;
            let mut prev_month = None;
            for col in 0..weeks {
                let (_, m, d) = civil_from_days(first + col as i64 * 7);
                let starts = prev_month.is_some_and(|p| p != m) || (prev_month.is_none() && d <= 7);
                prev_month = Some(m);
                if !starts {
                    continue;
                }
                let x = origin.x + col as f32 * pitch;
                let m_text = if cairn_i18n::is_chinese() {
                    format!("{m}月")
                } else {
                    match m {
                        1 => "Jan",
                        2 => "Feb",
                        3 => "Mar",
                        4 => "Apr",
                        5 => "May",
                        6 => "Jun",
                        7 => "Jul",
                        8 => "Aug",
                        9 => "Sep",
                        10 => "Oct",
                        11 => "Nov",
                        12 => "Dec",
                        _ => "",
                    }
                    .to_string()
                };
                let l = TextLayout::new(ts, &m_text, &small(10.5));
                let w = l.size().width;
                if x >= last_right + 6.0 && x + w <= origin.x + grid_w + 2.0 {
                    last_right = x + w;
                    self.months.push((x, l));
                }
            }
        }

        self.weekdays.clear();
        if labels {
            for row in [0usize, 2, 4] {
                let l = TextLayout::new(ts, weekday_label(row), &small(10.5));
                self.weekdays.push((origin.y + row as f32 * pitch + cell / 2.0, l));
            }
        }
        self.legend =
            geo.legend_y.map(|_| (TextLayout::new(ts, tr!("board.heatmap_less"), &small(10.5)), TextLayout::new(ts, tr!("board.heatmap_more"), &small(10.5))));
        self.geo = Some(geo);
        self.relabel_tip(ts);
    }

    /// Window coordinates of the first cell's top-left corner.
    fn origin(&self, geo: &Geo) -> Point {
        Point::new(self.content.x + geo.origin.x, self.content.y + geo.origin.y)
    }

    /// Window-space rect of a day's cell, or `None` if the day is off the graph or in the future.
    fn cell_rect(&self, geo: &Geo, day: i64) -> Option<Rect> {
        let i = day - geo.first;
        if i < 0 || day > self.daily.today {
            return None;
        }
        let (col, row) = ((i / 7) as usize, (i % 7) as usize);
        let o = self.origin(geo);
        (col < geo.weeks).then(|| Rect::new(o.x + col as f32 * geo.pitch, o.y + row as f32 * geo.pitch, geo.cell, geo.cell))
    }

    fn day_at(&self, p: Point) -> Option<i64> {
        let geo = self.geo?;
        let o = self.origin(&geo);
        let fx = (p.x - o.x) / geo.pitch;
        let fy = (p.y - o.y) / geo.pitch;
        if fx < 0.0 || !(0.0..7.0).contains(&fy) || fx >= geo.weeks as f32 {
            return None;
        }
        let day = geo.first + fx as i64 * 7 + fy as i64;
        (day <= self.daily.today).then_some(day)
    }

    fn relabel_tip(&mut self, ts: &mut TextSystem) {
        self.tip = self.hover.map(|day| {
            let (_, m, d) = civil_from_days(day);
            let h = self.daily.day(day);
            let amount = if h < 1.0 / 60.0 {
                tr!("board.no_play").to_string()
            } else if h < 1.0 {
                tr!("board.mins_count", mins = (h * 60.0).round())
            } else {
                tr!("board.hours_count", hours = format!("{h:.1}"))
            };
            let text = if cairn_i18n::is_chinese() {
                format!("{m}月{d}日 星期{} · {amount}", weekday_label(weekday(day)))
            } else {
                format!("{}/{}, {} · {amount}", m, d, weekday_label(weekday(day)))
            };
            TextLayout::new(ts, &text, &TextStyle { font_size: 12.0, line_height: 1.25, weight: 600, family: FontFamily::SansSerif })
        });
    }

    /// Tooltip rect: above the cell, or below when there is no room,
    /// and inside the widget horizontally.
    fn tip_rect(&self) -> Option<Rect> {
        let (geo, day, tip) = (self.geo?, self.hover?, self.tip.as_ref()?);
        let cell = self.cell_rect(&geo, day)?;
        let s = tip.size();
        let (w, h) = (s.width + 16.0, s.height + 10.0);
        let c = self.content;
        let x = (cell.center().x - w / 2.0).clamp(c.x, (c.right() - w).max(c.x));
        let above = cell.y - h - 6.0;
        let y = if above >= c.y - 2.0 { above } else { cell.bottom() + 6.0 };
        Some(Rect::new(x, y, w, h))
    }

    fn hover_damage(&self) -> Option<Rect> {
        let geo = self.geo?;
        let cell = self.cell_rect(&geo, self.hover?)?.inflate(4.0, 4.0);
        Some(match self.tip_rect() {
            Some(t) => cell.union(&t.inflate(12.0, 12.0)),
            None => cell,
        })
    }
}

into_el!(HeatmapView);

impl<S: 'static> Widget<S> for HeatmapView {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let same_days = new.daily.start == self.daily.start && new.daily.today == self.daily.today;
        let change = if !same_days {
            // A new day forces a relayout in `arrange` by clearing the cached key.
            self.geo_key = (0.0, 0.0, 0, 0);
            Change::LAYOUT
        } else if new.daily.hours != self.daily.hours || new.palette != self.palette {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.daily = new.daily;
        self.palette = new.palette;
        self.live = new.live;
        if !self.live {
            self.hover = None;
        }
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        Size::new(input.known_width.unwrap_or(0.0), input.known_height.unwrap_or(0.0))
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        // Geometry is relative to the top-left corner, so moving the widget does not need a
        // relayout. Only a size or date change does.
        self.content = content;
        let key = (content.width, content.height, self.daily.start, self.daily.hours.len());
        if key != self.geo_key {
            self.geo_key = key;
            self.layout(cx.text());
        }
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &cairn_ui::Theme) -> Rect {
        bounds.inflate(6.0, 6.0)
    }

    fn paint(&mut self, _cx: &mut PaintCx, p: &mut dyn Painter) {
        let Some(geo) = self.geo else { return };
        let (cx0, cy0) = (self.content.x, self.content.y);
        let o = self.origin(&geo);
        let radius = (geo.cell * 0.26).max(1.5);
        for col in 0..geo.weeks {
            for row in 0..7 {
                let day = geo.first + (col * 7 + row) as i64;
                if day > self.daily.today {
                    break;
                }
                let rect = Rect::new(o.x + col as f32 * geo.pitch, o.y + row as f32 * geo.pitch, geo.cell, geo.cell);
                p.fill_rounded_rect(rect, radius, self.palette[level(self.daily.day(day))]);
            }
        }
        if let Some(r) = self.cell_rect(&geo, self.daily.today) {
            p.stroke_rect(r.inflate(1.5, 1.5), radius + 1.5, 1.3, fg(0.75));
        }
        for (x, l) in &self.months {
            p.draw_text(l, Point::new(cx0 + x, cy0), fg(0.45));
        }
        for (y, l) in &self.weekdays {
            let s = l.size();
            p.draw_text(l, Point::new(cx0 + geo.labels_right - s.width, cy0 + y - s.height / 2.0), fg(0.38));
        }
        if let (Some(y), Some((less, more))) = (geo.legend_y.map(|y| cy0 + y), &self.legend) {
            let c = (geo.cell).min(10.0);
            let step = c + 3.0;
            let right = o.x + geo.weeks as f32 * geo.pitch - (geo.pitch - geo.cell);
            let ms = more.size();
            let mut x = right - ms.width;
            p.draw_text(more, Point::new(x, y - ms.height / 2.0), fg(0.4));
            x -= 6.0 + step * 5.0 - 3.0;
            for (i, color) in self.palette.iter().enumerate() {
                p.fill_rounded_rect(Rect::new(x + i as f32 * step, y - c / 2.0, c, c), (c * 0.26).max(1.5), *color);
            }
            let ls = less.size();
            p.draw_text(less, Point::new(x - 6.0 - ls.width, y - ls.height / 2.0), fg(0.4));
        }
        if let Some(day) = self.hover
            && let Some(r) = self.cell_rect(&geo, day)
        {
            p.stroke_rect(r.inflate(1.0, 1.0), radius + 1.0, 1.5, fg(0.95));
        }
        if let (Some(rect), Some(tip)) = (self.tip_rect(), &self.tip) {
            let light = current().light;
            p.draw_shadow(rect, 8.0, &Shadow::new(0.0, 4.0, 14.0, Color::rgbaf(0, 0, 0, if light { 0.16 } else { 0.4 })));
            p.fill_rounded_rect(rect, 8.0, if light { Color::rgbaf(255, 255, 255, 0.97) } else { Color::rgbaf(14, 16, 22, 0.94) });
            p.stroke_rect(rect, 8.0, 0.0, fg(0.12));
            p.draw_text(tip, Point::new(rect.x + 8.0, rect.y + 5.0), fg(0.92));
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if !self.live {
            return;
        }
        let hover = match event {
            Event::PointerMove { pos } => self.day_at(*pos),
            Event::PointerLeave => None,
            _ => return,
        };
        if hover == self.hover {
            return;
        }
        if let Some(r) = self.hover_damage() {
            cx.request_paint_rect(r);
        }
        self.hover = hover;
        self.relabel_tip(cx.text());
        if let Some(r) = self.hover_damage() {
            cx.request_paint_rect(r);
        }
    }
}
