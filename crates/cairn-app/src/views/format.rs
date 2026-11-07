//! Formatting for numbers, dates, relative times and durations.

use cairn_i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let n = n as f64;
    if n >= 1024.0 * MB {
        format!("{:.1} GB", n / (1024.0 * MB))
    } else if n >= MB {
        format!("{:.1} MB", n / MB)
    } else {
        format!("{:.0} KB", (n / 1024.0).max(1.0))
    }
}

pub fn speed(bps: u64) -> String {
    format!("{}/s", bytes(bps))
}

/// Formats a count with the unit style of the current language.
pub fn count(n: u64) -> String {
    match cairn_i18n::current_language() {
        cairn_i18n::Language::JaJp => {
            if n >= 100_000_000 {
                format!("{:.1} 億", n as f64 / 1e8)
            } else if n >= 10_000 {
                format!("{:.1} 万", n as f64 / 1e4)
            } else {
                n.to_string()
            }
        }
        cairn_i18n::Language::ZhCn | cairn_i18n::Language::Auto if cairn_i18n::is_chinese() => {
            if n >= 100_000_000 {
                format!("{:.1} 亿", n as f64 / 1e8)
            } else if n >= 10_000 {
                format!("{:.1} 万", n as f64 / 1e4)
            } else {
                n.to_string()
            }
        }
        _ => {
            if n >= 1_000_000 {
                format!("{:.1}M", n as f64 / 1e6)
            } else if n >= 1_000 {
                format!("{:.1}k", n as f64 / 1e3)
            } else {
                n.to_string()
            }
        }
    }
}

/// Converts Unix seconds to `YYYY-MM-DD` in UTC. Only the date is needed here.
pub fn date(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Relative time, such as just now, 5 minutes ago or 3 days ago. Older times show the date.
pub fn ago(unix: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = now.saturating_sub(unix);
    match s {
        0..60 => tr!("format.just_now").to_string(),
        60..3600 => tr!("format.mins_ago", n = s / 60),
        3600..86_400 => tr!("format.hours_ago", n = s / 3600),
        86_400..2_592_000 => tr!("format.days_ago", n = s / 86_400),
        _ => date(unix),
    }
}

/// Duration in the current language: seconds, minutes and seconds, or hours and minutes.
pub fn duration(secs: u64) -> String {
    match secs {
        0..60 => tr!("format.duration_secs", n = secs),
        60..3600 => tr!("format.duration_mins", m = secs / 60, s = secs % 60),
        _ => tr!("format.duration_hours", h = secs / 3600, m = secs % 3600 / 60),
    }
}

/// Takes only the date part of an ISO time such as `2024-06-13T08:24:03+00:00` from the manifest.
pub fn iso_date(s: &str) -> &str {
    s.get(..10).unwrap_or(s)
}

/// Converts a day count to a Gregorian year, month and day, using Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
