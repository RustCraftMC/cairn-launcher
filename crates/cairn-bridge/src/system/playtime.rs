//! Playtime tracking and GitHub-style activity heatmap data model.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use super::clock;

const PLAYTIME_FILE: &str = "stats/playtime.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaytimeDay {
    pub total_secs: u64,
    #[serde(default)]
    pub instances: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaytimeStore {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub records: HashMap<String, PlaytimeDay>,
}

fn default_version() -> u32 {
    1
}

impl Default for PlaytimeStore {
    fn default() -> Self {
        PlaytimeStore { version: 1, records: HashMap::new() }
    }
}

/// One column in the heatmap grid representing Monday through Sunday.
#[derive(Debug, Clone)]
pub struct WeekColumn {
    /// Playtime in hours for each day of the week (0: Monday, 6: Sunday).
    pub hours: [f32; 7],
    /// Date strings (`YYYY-MM-DD`) for each day of the week.
    pub dates: [String; 7],
}

#[derive(Debug, Clone)]
pub struct HeatmapMatrix {
    pub weeks: Vec<WeekColumn>,
    /// Tuples of `(week_index, month_name)` for axis labels.
    pub month_labels: Vec<(usize, String)>,
    pub total_hours: f32,
    pub streak_days: u32,
    pub active_days: usize,
    pub most_played: Option<String>,
}

impl PlaytimeStore {
    /// Loads playtime stats from `<dir>/stats/playtime.json`.
    /// Returns default empty records if the file is missing or corrupted without creating directories.
    pub fn load(dir: &Path) -> Self {
        fs::read(dir.join(PLAYTIME_FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        let _ = Self::save_to(dir, self);
    }

    fn save_to(dir: &Path, store: &PlaytimeStore) -> std::io::Result<()> {
        let path = dir.join(PLAYTIME_FILE);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(store)?;
        fs::write(path, data)
    }

    pub fn today_string() -> String {
        let t = clock::local_now();
        format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
    }

    pub fn record(&mut self, instance: &str, secs: u64) {
        if secs == 0 {
            return;
        }
        let today = Self::today_string();
        let day = self.records.entry(today).or_default();
        day.total_secs += secs;
        *day.instances.entry(instance.to_string()).or_default() += secs;
    }

    pub fn total_hours(&self) -> f32 {
        let secs: u64 = self.records.values().map(|d| d.total_secs).sum();
        (secs as f32 / 3600.0 * 10.0).round() / 10.0
    }

    pub fn streak_days(&self) -> u32 {
        let mut keys: Vec<&String> = self.records.keys().filter(|k| self.records[*k].total_secs >= 60).collect();
        keys.sort();
        let mut max_streak = 0;
        let mut cur_streak = 0;
        let mut prev_day: Option<i64> = None;

        for k in keys {
            if let Some((y, m, d)) = parse_date(k) {
                let day_serial = date_to_serial(y, m, d);
                if let Some(prev) = prev_day {
                    if day_serial == prev + 1 {
                        cur_streak += 1;
                    } else if day_serial > prev + 1 {
                        cur_streak = 1;
                    }
                } else {
                    cur_streak = 1;
                }
                prev_day = Some(day_serial);
                max_streak = max_streak.max(cur_streak);
            }
        }
        max_streak
    }

    pub fn most_played(&self) -> Option<String> {
        let mut counts: HashMap<String, u64> = HashMap::new();
        for d in self.records.values() {
            for (k, s) in &d.instances {
                *counts.entry(k.clone()).or_default() += *s;
            }
        }
        counts.into_iter().max_by_key(|(_, s)| *s).map(|(k, _)| k)
    }

    /// Generates a heatmap matrix for the trailing `num_weeks` ending with the current week.
    pub fn heatmap_matrix(&self, num_weeks: usize) -> HeatmapMatrix {
        let t = clock::local_now();
        let today_serial = date_to_serial(t.year as i32, t.month as i32, t.day as i32);
        let today_dow = weekday_of_serial(today_serial);

        let start_serial = today_serial - today_dow as i64 - ((num_weeks - 1) * 7) as i64;

        let mut weeks = Vec::with_capacity(num_weeks);
        let mut month_labels = Vec::new();
        let mut last_month = -1;
        let mut last_label_week = -999;

        for w in 0..num_weeks {
            let mut col = WeekColumn { hours: [0.0; 7], dates: std::array::from_fn(|_| String::new()) };
            for d in 0..7 {
                let cur_serial = start_serial + (w * 7 + d) as i64;
                let (y, m, day) = serial_to_date(cur_serial);
                let date_str = format!("{:04}-{:02}-{:02}", y, m, day);
                col.dates[d] = date_str.clone();

                if cur_serial <= today_serial {
                    if let Some(record) = self.records.get(&date_str) {
                        col.hours[d] = (record.total_secs as f32 / 3600.0 * 10.0).round() / 10.0;
                    }
                }

                if m != last_month {
                    last_month = m;
                    // Only place a month label when spaced at least 3 weeks apart and not too close to the end.
                    if (w as i32 - last_label_week >= 3) && (w + 2 <= num_weeks) {
                        month_labels.push((w, format!("{}月", m)));
                        last_label_week = w as i32;
                    }
                }
            }
            weeks.push(col);
        }

        let active_days = self.records.values().filter(|r| r.total_secs >= 60).count();

        HeatmapMatrix { weeks, month_labels, total_hours: self.total_hours(), streak_days: self.streak_days(), active_days, most_played: self.most_played() }
    }
}

fn parse_date(s: &str) -> Option<(i32, i32, i32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        let y = parts[0].parse().ok()?;
        let m = parts[1].parse().ok()?;
        let d = parts[2].parse().ok()?;
        Some((y, m, d))
    } else {
        None
    }
}

/// Converts a Gregorian calendar date to a continuous serial day number.
fn date_to_serial(year: i32, month: i32, day: i32) -> i64 {
    let mut y = year;
    let mut m = month;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let a = (y / 100) as i64;
    let b = 2 - a + a / 4;
    (365.25 * (y + 4716) as f64).floor() as i64 + (30.6001 * (m + 1) as f64).floor() as i64 + day as i64 + b - 1524
}

fn serial_to_date(serial: i64) -> (i32, i32, i32) {
    let z = serial + 1524;
    let a = ((z as f64 - 1867216.25) / 36524.25).floor() as i64;
    let a = z + 1 + a - a / 4;
    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25).floor() as i64;
    let d = (365.25 * c as f64).floor() as i64;
    let e = ((b - d) as f64 / 30.6001).floor() as i64;
    let day = b - d - (30.6001 * e as f64).floor() as i64;
    let month = if e < 14 { e - 1 } else { e - 13 };
    let year = if month > 2 { c - 4716 } else { c - 4715 };
    (year as i32, month as i32, day as i32)
}

/// Day of week from serial date: 0 for Monday through 6 for Sunday.
fn weekday_of_serial(serial: i64) -> usize {
    ((serial + 1) % 7) as usize
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Generates a non-existent temporary directory path with a unique PID and timestamp.
    fn missing_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        std::env::temp_dir().join(format!("cairn-playtime-{tag}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn load_missing_dir_returns_empty_and_writes_nothing() {
        let dir = missing_temp_dir("missing");
        let store = PlaytimeStore::load(&dir);
        assert!(store.records.is_empty());
        assert_eq!(store.version, 1);
        assert!(!dir.exists(), "load 不应该创建目录或文件");
    }

    #[test]
    fn load_returns_stored_records_as_they_are() {
        let dir = missing_temp_dir("stored");
        fs::create_dir_all(dir.join("stats")).unwrap();
        fs::write(dir.join(PLAYTIME_FILE), r#"{"version":1,"records":{"2026-10-01":{"total_secs":3600,"instances":{"a":3600}}}}"#).unwrap();
        let store = PlaytimeStore::load(&dir);
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.records["2026-10-01"].total_secs, 3600);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_records_are_not_replaced_with_samples() {
        let dir = missing_temp_dir("empty");
        fs::create_dir_all(dir.join("stats")).unwrap();
        fs::write(dir.join(PLAYTIME_FILE), r#"{"version":1,"records":{}}"#).unwrap();
        let store = PlaytimeStore::load(&dir);
        assert!(store.records.is_empty());
        assert_eq!(fs::read_to_string(dir.join(PLAYTIME_FILE)).unwrap(), r#"{"version":1,"records":{}}"#);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_returns_empty_and_is_kept() {
        let dir = missing_temp_dir("corrupt");
        fs::create_dir_all(dir.join("stats")).unwrap();
        fs::write(dir.join(PLAYTIME_FILE), "{ broken").unwrap();
        let store = PlaytimeStore::load(&dir);
        assert!(store.records.is_empty());
        assert_eq!(fs::read_to_string(dir.join(PLAYTIME_FILE)).unwrap(), "{ broken");
        fs::remove_dir_all(&dir).unwrap();
    }
}
