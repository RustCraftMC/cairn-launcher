//! Built-in widgets: the registry, plus the look and data helpers they share.
//! Each widget is one file exporting a [`Kind`] with its name, icon, sizes, paint and menu options.
//! New widgets get a file and a line in [`ALL`]. YAML widgets live in [`custom`].

use std::collections::{BTreeMap, HashMap};

use cairn_bridge::{LocalTime, PlaytimeStore};
use cairn_i18n::tr;
use cairn_ui::{El, Icon, Insets, Panel, Styled, label, panel, row};

use super::glyph::{Glyph, glyph};
use crate::gui::Gui;
use crate::style::{accent, fg};

mod activity;
mod clock;
pub mod custom;
mod greeting;
mod heatmap;
mod launch;
mod quote;
mod shortcuts;
mod stats;

/// Size in grid cells: `w` columns by `h` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub w: u16,
    pub h: u16,
}

pub const fn span(w: u16, h: u16) -> Span {
    Span { w, h }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Game,
    Stats,
    Info,
    /// Widgets the user writes in YAML under `<data dir>/widgets`.
    Custom,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Game, Category::Stats, Category::Info, Category::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Category::Game => tr!("board.cat_game"),
            Category::Stats => tr!("board.cat_stats"),
            Category::Info => tr!("board.cat_info"),
            Category::Custom => tr!("board.cat_custom"),
        }
    }
}

pub struct Preset {
    pub label: &'static str,
    pub span: Span,
}

/// Per-widget options shown in the context menu.
/// Values are stored as strings in the layout entry's `options`.
pub enum Opt {
    /// Pick one value, shown as a row of labels. `choices` holds (value, name) pairs.
    Choice { key: &'static str, label: &'static str, choices: &'static [(&'static str, &'static str)], default: &'static str },
    Toggle { key: &'static str, label: &'static str, default: bool },
    Action { label: &'static str, icon: Icon, run: fn(&mut Gui) },
}

pub struct Kind {
    /// Kind name saved in layout files; keep it stable.
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub glyph: Glyph,
    pub category: Category,
    pub min: Span,
    pub max: Span,
    pub presets: &'static [Preset],
    /// Index into `presets` used when the widget is first added.
    pub default: usize,
    pub render: fn(&Gui, &Ctx) -> El<Gui>,
    pub options: &'static [Opt],
}

impl Preset {
    pub fn label(&self) -> &'static str {
        match self.label {
            "Small" | "小" => tr!("board.clock_small"),
            "Medium" | "中" => tr!("board.clock_medium"),
            "Large" | "大" => tr!("board.clock_large"),
            "Compact" | "紧凑" => tr!("board.activity_compact"),
            "UltraNarrow" | "极窄" => tr!("board.activity_ultra_narrow"),
            "Narrow" | "窄" => tr!("board.quote_narrow"),
            "Wide" | "宽" => tr!("board.quote_wide"),
            "Full" | "全宽" => tr!("board.quote_full"),
            other => other,
        }
    }
}

pub fn opt_label(s: &str) -> &'static str {
    match s {
        "Show Seconds" | "seconds" => tr!("board.clock_show_seconds"),
        "Refresh Quote" | "refresh_quote" => tr!("board.quote_refresh"),
        "Reload YAML" | "reload_yaml" => tr!("board.custom_reload"),
        "Dial" | "face" => tr!("board.clock_title"),
        "Palette" | "palette" => tr!("settings.theme_mode"),
        _other => "",
    }
}

pub fn opt_choice_label(val: &str) -> &'static str {
    match val {
        "digital" => tr!("board.clock_digital"),
        "auto" => tr!("board.clock_auto"),
        "analog" => tr!("board.clock_analog"),
        "flip" => tr!("board.clock_flip"),
        "light" => tr!("board.heatmap_light"),
        "dark" => tr!("board.heatmap_dark"),
        "system" => tr!("board.heatmap_auto"),
        _ => "",
    }
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self.id {
            "activity" => tr!("board.activity_title"),
            "clock" => tr!("board.clock_title"),
            "custom" => tr!("board.custom_title"),
            "greeting" => tr!("board.greeting_title"),
            "heatmap" => tr!("board.heatmap_title"),
            "launch" => tr!("board.launch_title"),
            "quote" => tr!("board.quote_title"),
            "shortcuts" => tr!("board.shortcuts_title"),
            "stats" => tr!("board.stats_title"),
            _ => self.name,
        }
    }

    pub fn summary(&self) -> &'static str {
        match self.id {
            "activity" => tr!("board.activity_desc"),
            "clock" => tr!("board.clock_desc"),
            "greeting" => tr!("board.greeting_desc"),
            "heatmap" => tr!("board.heatmap_desc"),
            "launch" => tr!("board.launch_desc"),
            "quote" => tr!("board.quote_desc"),
            "shortcuts" => tr!("board.shortcuts_desc"),
            "stats" => tr!("board.stats_desc"),
            _ => self.summary,
        }
    }

    pub fn clamp(&self, s: Span) -> Span {
        span(s.w.clamp(self.min.w, self.max.w), s.h.clamp(self.min.h, self.max.h))
    }
}

/// All built-in widgets. This order is also the order in the widget library.
pub const ALL: &[&Kind] = &[&greeting::KIND, &clock::KIND, &launch::KIND, &activity::KIND, &shortcuts::KIND, &heatmap::KIND, &stats::KIND, &quote::KIND];

pub fn find(id: &str) -> Option<&'static Kind> {
    ALL.iter().copied().find(|k| k.id == id).or_else(|| custom::find(id))
}

/// Every widget listed in the library: built-in ones first, then custom ones.
pub fn all() -> Vec<&'static Kind> {
    ALL.iter().copied().chain(custom::all()).collect()
}

pub struct Ctx<'a> {
    pub kind: &'static Kind,
    pub span: Span,
    pub options: &'a BTreeMap<String, String>,
    /// Library previews and drag ghosts: not interactive, no self-animating controls.
    /// Sample data is used when there is no real data.
    pub preview: bool,
}

impl Ctx<'_> {
    /// Current value of a choice option, or its default when unset.
    pub fn opt(&self, key: &str) -> &str {
        if let Some(v) = self.options.get(key) {
            return v;
        }
        self.kind
            .options
            .iter()
            .find_map(|o| match o {
                Opt::Choice { key: k, default, .. } if *k == key => Some(*default),
                _ => None,
            })
            .unwrap_or("")
    }

    pub fn flag(&self, key: &str) -> bool {
        match self.options.get(key) {
            Some(v) => v == "true",
            None => self
                .kind
                .options
                .iter()
                .find_map(|o| match o {
                    Opt::Toggle { key: k, default, .. } if *k == key => Some(*default),
                    _ => None,
                })
                .unwrap_or(false),
        }
    }
}

pub fn card() -> Panel<Gui> {
    panel().hover_effect(false).fill_width().fill_height().padding(Insets::all(14.0)).gap(8.0)
}

pub fn heading(g: Glyph, title: impl Into<String>) -> El<Gui> {
    row().gap(6.0).child(glyph(g).size(14.0).color(accent())).child(label(title).font_size(12.5).weight(700).color(fg(0.58)).truncate()).into()
}

pub fn figure(value: impl Into<String>, unit: &str, size: f32) -> El<Gui> {
    row()
        .gap(4.0)
        .align(cairn_ui::Align::End)
        .child(label(value).font_size(size).weight(700).color(fg(0.95)))
        .child(label(unit).font_size(12.0).weight(600).color(fg(0.5)).margin(Insets::new(0.0, 0.0, size * 0.16, 0.0)))
        .into()
}

pub fn hours(h: f32) -> String {
    if h < 10.0 { format!("{:.1}", h) } else { format!("{:.0}", h) }
}

pub fn weekday_label(idx: usize) -> &'static str {
    match idx % 7 {
        0 => tr!("board.days_mon"),
        1 => tr!("board.days_tue"),
        2 => tr!("board.days_wed"),
        3 => tr!("board.days_thu"),
        4 => tr!("board.days_fri"),
        5 => tr!("board.days_sat"),
        _ => tr!("board.days_sun"),
    }
}

/// Days since 1970-01-01 for a Gregorian date, using Howard Hinnant's algorithm.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

/// Day of the week, with 0 for Monday.
pub fn weekday(day: i64) -> usize {
    // 1970-01-01 was a Thursday, so the offset is 3.
    (day + 3).rem_euclid(7) as usize
}

/// Local calendar day of `t`, as days since 1970-01-01.
pub fn today(t: &LocalTime) -> i64 {
    days_from_civil(i32::from(t.year), u32::from(t.month), u32::from(t.day))
}

/// Parses a playtime record date, `YYYY-MM-DD`, into days since 1970-01-01.
pub fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

/// Relative label: today, yesterday, or N days ago. Older days show month and day.
pub fn relative_day(day: i64, today: i64) -> String {
    match today - day {
        0 => tr!("board.today").to_string(),
        1 => tr!("board.yesterday").to_string(),
        n @ 2..=6 => tr!("board.days_ago", days = n),
        _ => {
            let (_, m, d) = civil_from_days(day);
            if cairn_i18n::is_chinese() { format!("{m}月{d}日") } else { format!("{m}/{d}") }
        }
    }
}

/// Playtime per day in hours, from a Monday through today. The heatmap and stats read from here.
pub struct Daily {
    /// Today, in days since 1970-01-01.
    pub today: i64,
    /// Day of `hours[0]`, always a Monday.
    pub start: i64,
    pub hours: Vec<f32>,
    /// Total playtime of all records, including days outside the window.
    pub total: f32,
}

impl Daily {
    /// The last `weeks` weeks, including the current one.
    pub fn from_store(store: &PlaytimeStore, now: &LocalTime, weeks: usize) -> Daily {
        let today = today(now);
        let start = today - weekday(today) as i64 - (weeks.max(1) as i64 - 1) * 7;
        let mut hours = vec![0.0; (today - start + 1) as usize];
        let mut total = 0.0;
        for (key, day) in &store.records {
            let h = day.total_secs as f32 / 3600.0;
            total += h;
            if let Some(d) = parse_day(key)
                && (start..=today).contains(&d)
            {
                hours[(d - start) as usize] += h;
            }
        }
        Daily { today, start, hours, total }
    }

    /// Sample data for library previews. It stays in memory and is never saved.
    pub fn sample(now: &LocalTime, weeks: usize) -> Daily {
        let today = today(now);
        let start = today - weekday(today) as i64 - (weeks.max(1) as i64 - 1) * 7;
        let hours: Vec<f32> = (start..=today)
            .map(|d| {
                let r = ((d.wrapping_mul(9301) + 49_297).rem_euclid(233_280)) as f32 / 233_280.0;
                let weekend = weekday(d) >= 5;
                match (weekend, r) {
                    (true, r) if r < 0.75 => 1.0 + r * 4.6,
                    (false, r) if r < 0.45 => 0.3 + r * 3.0,
                    _ => 0.0,
                }
            })
            .collect();
        let total = hours.iter().sum::<f32>() * 1.6;
        Daily { today, start, hours, total }
    }

    pub fn is_empty(&self) -> bool {
        self.hours.iter().all(|h| *h <= 0.0)
    }

    /// Hours played on day `d`, or 0 outside the window.
    pub fn day(&self, d: i64) -> f32 {
        if d < self.start || d > self.today { 0.0 } else { self.hours[(d - self.start) as usize] }
    }

    /// Total hours from `from` to `to`, inclusive on both ends.
    pub fn sum(&self, from: i64, to: i64) -> f32 {
        (from..=to).map(|d| self.day(d)).sum()
    }

    /// Consecutive days with play, counted back from today.
    /// If nothing is played today yet, counting starts at yesterday.
    pub fn streak(&self) -> u32 {
        let played = |d: i64| self.day(d) >= 1.0 / 60.0;
        let mut d = if played(self.today) { self.today } else { self.today - 1 };
        let mut n = 0;
        while d >= self.start && played(d) {
            n += 1;
            d -= 1;
        }
        n
    }

    /// Days with at least one minute of play inside the window.
    pub fn active_days(&self) -> usize {
        self.hours.iter().filter(|h| **h >= 1.0 / 60.0).count()
    }
}

/// Recently played instances as (instance ID, last played day), most recent first.
pub fn recent_instances(store: &PlaytimeStore) -> Vec<(String, i64)> {
    let mut last: HashMap<&str, i64> = HashMap::new();
    for (key, day) in &store.records {
        let Some(d) = parse_day(key) else { continue };
        for (inst, secs) in &day.instances {
            if *secs > 0 {
                let e = last.entry(inst.as_str()).or_insert(d);
                *e = (*e).max(d);
            }
        }
    }
    let mut out: Vec<(String, i64)> = last.into_iter().map(|(k, d)| (k.to_string(), d)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

pub fn most_played(store: &PlaytimeStore) -> Option<(String, f32)> {
    let mut sum: HashMap<&str, u64> = HashMap::new();
    for day in store.records.values() {
        for (inst, secs) in &day.instances {
            *sum.entry(inst.as_str()).or_default() += secs;
        }
    }
    sum.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0))).map(|(k, s)| (k.to_string(), s as f32 / 3600.0))
}

/// UI name for an instance ID: the instance name if found, else the last segment of the ID.
pub fn instance_label(g: &Gui, id: &str) -> String {
    match g.m.instance(id) {
        Some(i) => i.name.clone(),
        None => id.rsplit('/').next().unwrap_or(id).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip_and_weekday() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 30)), (2026, 10, 30));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        // 2026-10-30 is a Friday, so the index is 4.
        assert_eq!(weekday(days_from_civil(2026, 10, 30)), 4);
        assert_eq!(parse_day("2026-10-30"), Some(days_from_civil(2026, 10, 30)));
        assert_eq!(parse_day("invalid-date"), None);
    }

    #[test]
    fn registry_presets_respect_limits() {
        for k in ALL {
            assert!(!k.presets.is_empty() && k.default < k.presets.len(), "{}", k.id);
            for p in k.presets {
                assert_eq!(k.clamp(p.span), p.span, "{}: preset {} out of range", k.id, p.label);
                assert!(p.span.w <= super::super::grid::COLS && p.span.h <= super::super::grid::ROWS);
            }
            assert_eq!(find(k.id).map(|f| f.name), Some(k.name));
        }
        let mut ids: Vec<&str> = ALL.iter().map(|k| k.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len(), "duplicate kind id");
    }

    #[test]
    fn streak_counts_back_from_today_or_yesterday() {
        let mut d = Daily { today: 100, start: 93, hours: vec![0.0; 8], total: 0.0 };
        d.hours[5] = 1.0; // 98
        d.hours[6] = 0.5; // 99
        assert_eq!(d.streak(), 2, "streak test");
        d.hours[7] = 2.0; // 100
        assert_eq!(d.streak(), 3);
        assert_eq!(d.active_days(), 3);
        assert!((d.sum(98, 100) - 3.5).abs() < 1e-4);
    }
}
