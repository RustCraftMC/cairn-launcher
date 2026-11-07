//! Clock widget: an analog dial when the area is near square, large digits when it is wide.
//! The dial redraws on its own timer, once per second (or per minute without seconds).
//! It skips app state, so ticking does not rebuild the whole UI.

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use cairn_bridge::{LocalTime, local_now};
use cairn_ui::{
    Change, Color, Easing, El, Event, EventCx, FontFamily, Insets, LayoutCx, MeasureInput, PaintCx, Painter, Point, Rect, Shadow, Size, Style, Styled,
    TextLayout, TextStyle, TextSystem, Transition, Widget,
};

use super::{Category, Ctx, Kind, Opt, Preset, card, span, weekday_label};
use crate::board::glyph::Glyph;
use crate::gui::Gui;
use crate::style::{accent, current, fg};

pub const KIND: Kind = Kind {
    id: "clock",
    name: "Clock",
    summary: "Analog or digital clock, switches with size",
    glyph: Glyph::Clock,
    category: Category::Info,
    min: span(2, 1),
    max: span(6, 4),
    presets: &[Preset { label: "Small", span: span(2, 2) }, Preset { label: "Medium", span: span(4, 2) }, Preset { label: "Large", span: span(4, 4) }],
    default: 0,
    render: view,
    options: &[
        Opt::Choice { key: "face", label: "Dial", choices: &[("auto", "Auto"), ("analog", "Analog"), ("digital", "Digital")], default: "auto" },
        Opt::Toggle { key: "seconds", label: "Seconds", default: true },
    ],
};

fn view(_g: &Gui, cx: &Ctx) -> El<Gui> {
    let face = match cx.opt("face") {
        "analog" => Face::Analog,
        "digital" => Face::Digital,
        _ => Face::Auto,
    };
    card().padding(Insets::all(10.0)).child(ClockFace::new(face, cx.flag("seconds"), !cx.preview).fill_width().grow(1.0).min_height(0.0)).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    Auto,
    Analog,
    Digital,
}

struct ClockFace {
    style: Style,
    key: Option<u64>,
    face: Face,
    seconds: bool,
    /// Previews do not tick: they are scaled, so self-redrawn dirty rects would not line up.
    live: bool,
    now: Option<LocalTime>,
    content: Rect,
    /// Font size of the digital face, which `relayout` shrinks until the time fits the width.
    font: f32,
    time: Option<TextLayout>,
    secs: Option<TextLayout>,
    date: Option<TextLayout>,
    numerals: Vec<(u8, TextLayout)>,
    /// Second hand as seconds since midnight, eased over a short transition when it steps.
    second: Transition,
    started: bool,
    /// Set by `paint`, cleared on each tick. A tick only requests a repaint when set, so a hidden
    /// clock (under a panel or fading out) does not repaint every second. The time still updates.
    painted: bool,
}

impl ClockFace {
    fn new(face: Face, seconds: bool, live: bool) -> Self {
        ClockFace {
            style: Style::default(),
            key: None,
            face,
            seconds,
            live,
            now: None,
            content: Rect::ZERO,
            font: 0.0,
            time: None,
            secs: None,
            date: None,
            numerals: Vec::new(),
            second: Transition::default(),
            started: false,
            painted: false,
        }
    }

    fn analog(&self) -> bool {
        match self.face {
            Face::Analog => true,
            Face::Digital => false,
            Face::Auto => self.content.width <= self.content.height * 1.45,
        }
    }

    fn radius(&self) -> f32 {
        (self.content.width.min(self.content.height) / 2.0 - 1.0).max(0.0)
    }

    fn relayout(&mut self, ts: &mut TextSystem) {
        let Some(t) = self.now else { return };
        let style = |size: f32, weight: u16| TextStyle { font_size: size, line_height: 1.2, weight, family: FontFamily::SansSerif };
        let set = |slot: &mut Option<TextLayout>, ts: &mut TextSystem, s: &str, st: &TextStyle| match slot {
            Some(l) => {
                l.set_style(ts, st);
                l.set_text(ts, s);
            }
            None => *slot = Some(TextLayout::new(ts, s, st)),
        };
        if self.analog() {
            let r = self.radius();
            let date = format!("{} {}", weekday_label(t.weekday as usize % 7), t.day);
            set(&mut self.date, ts, &date, &style((r * 0.15).clamp(9.0, 18.0), 700));
            let which: &[u8] = if r >= 92.0 {
                &[12, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
            } else if r >= 58.0 {
                &[12, 3, 6, 9]
            } else {
                &[]
            };
            let size = (r * 0.17).clamp(9.0, 20.0);
            if self.numerals.len() != which.len() || self.numerals.first().is_some_and(|(_, l)| (l.style().font_size - size).abs() > 0.1) {
                self.numerals = which.iter().map(|n| (*n, TextLayout::new(ts, &n.to_string(), &style(size, 600)))).collect();
            }
        } else {
            let c = self.content;
            let secs_w = if self.seconds { 1.35 } else { 0.0 };
            // A digit is about 0.6 font sizes wide. The text 21:47 is about 2.9 font sizes,
            // and the seconds about 1.35 (counted at the small size).
            let mut font = (c.height * 0.5).min(c.width / (3.0 + secs_w * 0.45)).clamp(16.0, 132.0);
            for _ in 0..2 {
                set(&mut self.time, ts, &format!("{:02}:{:02}", t.hour, t.minute), &style(font, 600));
                set(&mut self.secs, ts, &format!("{:02}", t.second), &style(font * 0.4, 600));
                let w = self.time.as_ref().map_or(0.0, |l| l.size().width)
                    + if self.seconds { self.secs.as_ref().map_or(0.0, |l| l.size().width) + font * 0.08 } else { 0.0 };
                if w <= c.width || font <= 16.0 {
                    break;
                }
                font = (font * c.width / w).max(16.0);
            }
            self.font = font;
            let date = if cairn_i18n::is_chinese() {
                if cairn_i18n::is_chinese() {
                    if cairn_i18n::is_chinese() {
                        format!("{}月{}日 星期{}", t.month, t.day, weekday_label(t.weekday as usize % 7))
                    } else {
                        format!("{}, {}/{}", weekday_label(t.weekday as usize % 7), t.month, t.day)
                    }
                } else {
                    format!("{}, {}/{}", weekday_label(t.weekday as usize % 7), t.month, t.day)
                }
            } else {
                format!("{}, {}/{}", weekday_label(t.weekday as usize % 7), t.month, t.day)
            };
            set(&mut self.date, ts, &date, &style((font * 0.2).clamp(11.0, 18.0), 600));
        }
    }

    fn day_seconds(t: &LocalTime) -> f32 {
        f32::from(t.hour) * 3600.0 + f32::from(t.minute) * 60.0 + f32::from(t.second)
    }

    /// Next redraw time: the next second, or the next minute when seconds are hidden.
    fn next_tick(&self, now: Instant, t: &LocalTime) -> Instant {
        let ms = if self.seconds { 1000 - u64::from(t.millis) } else { (59 - u64::from(t.second)) * 1000 + 1000 - u64::from(t.millis) };
        now + Duration::from_millis(ms + 8)
    }

    fn paint_analog(&self, p: &mut dyn Painter, now: Instant) {
        let Some(t) = self.now else { return };
        let r = self.radius();
        if r < 8.0 {
            return;
        }
        let light = current().light;
        let c = self.content.center();
        let dial = Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        p.fill_rounded_rect(dial, r, if light { Color::rgbaf(255, 255, 255, 0.55) } else { Color::rgbaf(0, 0, 0, 0.24) });
        p.draw_inner_shadow(dial, r, &Shadow::new(0.0, p.hairline(), 0.0, Color::WHITE.with_opacity(if light { 0.7 } else { 0.08 })));
        p.stroke_rect(dial, r, 0.0, fg(0.10));

        let at = |angle: f32, len: f32| Point::new(c.x + angle.sin() * len, c.y - angle.cos() * len);
        for i in 0..60 {
            let a = i as f32 / 60.0 * TAU;
            if i % 5 == 0 {
                p.draw_line(at(a, r * 0.80), at(a, r * 0.90), (r * 0.03).max(1.4), fg(0.72));
            } else if r >= 52.0 {
                p.draw_line(at(a, r * 0.86), at(a, r * 0.90), 1.0, fg(0.26));
            }
        }
        for (n, l) in &self.numerals {
            let s = l.size();
            let pt = at(f32::from(*n) / 12.0 * TAU, r * 0.64);
            p.draw_text(l, Point::new(pt.x - s.width / 2.0, pt.y - s.height / 2.0), fg(0.82));
        }
        if r >= 40.0
            && let Some(l) = &self.date
        {
            let s = l.size();
            p.draw_text(l, Point::new(c.x - s.width / 2.0, c.y + r * 0.34 - s.height / 2.0), accent());
        }

        let hour = (f32::from(t.hour % 12) + f32::from(t.minute) / 60.0) / 12.0 * TAU;
        let minute = (f32::from(t.minute) + f32::from(t.second) / 60.0) / 60.0 * TAU;
        let hands = fg(0.94);
        p.draw_line(at(hour + std::f32::consts::PI, r * 0.08), at(hour, r * 0.5), (r * 0.07).max(2.5), hands);
        p.draw_line(at(minute + std::f32::consts::PI, r * 0.08), at(minute, r * 0.76), (r * 0.045).max(1.8), hands);
        if self.seconds {
            let s = self.second.value(now).rem_euclid(60.0) / 60.0 * TAU;
            p.draw_line(at(s + std::f32::consts::PI, r * 0.16), at(s, r * 0.84), (r * 0.018).max(1.2), accent());
        }
        let dot = (r * 0.05).max(2.5);
        p.fill_rounded_rect(Rect::new(c.x - dot, c.y - dot, dot * 2.0, dot * 2.0), dot, accent());
        let pin = dot * 0.4;
        p.fill_rounded_rect(Rect::new(c.x - pin, c.y - pin, pin * 2.0, pin * 2.0), pin, if light { Color::WHITE } else { Color::rgb(20, 22, 28) });
    }

    fn paint_digital(&self, p: &mut dyn Painter) {
        let (Some(time), Some(date)) = (&self.time, &self.date) else { return };
        let c = self.content;
        let ts = time.size();
        let ds = date.size();
        let secs = self.secs.as_ref().filter(|_| self.seconds);
        let sw = secs.map_or(0.0, |l| l.size().width + self.font * 0.08);
        let block_h = ts.height + ds.height * 0.9;
        let x = c.x + (c.width - ts.width - sw) / 2.0;
        let y = c.y + (c.height - block_h) / 2.0;
        p.draw_text(time, Point::new(x, y), fg(0.96));
        if let Some(l) = secs {
            // Align the seconds with the bottom of the hours and minutes digits.
            let off = ts.height * 0.86 - l.size().height * 0.86;
            p.draw_text(l, Point::new(x + ts.width + self.font * 0.08, y + off), accent());
        }
        p.draw_text(date, Point::new(c.x + (c.width - ds.width) / 2.0, y + ts.height - ds.height * 0.05), fg(0.6));
    }
}

into_el!(ClockFace);

impl<S: 'static> Widget<S> for ClockFace {
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change {
        let changed = new.face != self.face || new.seconds != self.seconds;
        self.face = new.face;
        self.seconds = new.seconds;
        self.live = new.live;
        if changed {
            self.relayout(cx.text());
            Change::PAINT
        } else {
            Change::NONE
        }
    }

    fn measure(&mut self, _cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        Size::new(input.known_width.unwrap_or(0.0), input.known_height.unwrap_or(0.0))
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        let first = self.now.is_none();
        if first {
            let t = local_now();
            self.second.set(Self::day_seconds(&t));
            self.now = Some(t);
        }
        if first || content.width != self.content.width || content.height != self.content.height {
            self.content = content;
            self.relayout(cx.text());
        }
        self.content = content;
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        if !self.started && self.live {
            self.started = true;
            if let Some(t) = self.now {
                cx.request_timer(self.next_tick(now, &t));
            }
        }
        if self.second.is_animating(now) {
            cx.request_frame();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        self.painted = true;
        if self.analog() {
            self.paint_analog(p, cx.now());
        } else {
            self.paint_digital(p);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if *event != Event::Timer || !self.live {
            return;
        }
        let t = local_now();
        let now = cx.now();
        let target = Self::day_seconds(&t);
        let from = self.second.value(now);
        // Jumps under 3 seconds are animated. Midnight wrap-around or a longer gap (such as waking
        // from sleep) snaps the hand to its new position.
        if (0.0..3.0).contains(&(target - from)) {
            self.second.animate_to(target, now, Duration::from_millis(170), Easing::EaseOut);
        } else {
            self.second.set(target);
        }
        let old = self.now.replace(t);
        if old.is_none_or(|o| o.minute != t.minute || o.second != t.second || o.day != t.day) {
            self.relayout(cx.text());
        }
        if std::mem::take(&mut self.painted) {
            cx.request_paint();
        }
        cx.request_timer(self.next_tick(now, &t));
    }
}
