//! Reads the local time of this machine, in the system time zone.

use crate::LocalTime;

#[cfg(windows)]
use windows_sys::Win32::Foundation::SYSTEMTIME;
#[cfg(windows)]
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// Current local time from `GetLocalTime`, which uses the Windows system time zone.
#[cfg(windows)]
pub fn local_now() -> LocalTime {
    let mut st = SYSTEMTIME::default();
    // SAFETY: `st` is a writable `SYSTEMTIME`, and `GetLocalTime` only writes the time into it.
    unsafe { GetLocalTime(&mut st) };
    LocalTime {
        year: st.wYear,
        month: st.wMonth as u8,
        day: st.wDay as u8,
        weekday: monday_first(st.wDayOfWeek),
        hour: st.wHour as u8,
        minute: st.wMinute as u8,
        second: st.wSecond as u8,
        millis: st.wMilliseconds,
    }
}

/// Current local time in the system time zone using `localtime_r`.
#[cfg(not(windows))]
pub fn local_now() -> LocalTime {
    let now = std::time::SystemTime::now();
    let duration = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;
    let millis = duration.subsec_millis() as u16;

    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    unsafe {
        libc::localtime_r(&secs, tm.as_mut_ptr());
    }
    let tm = unsafe { tm.assume_init() };

    LocalTime {
        year: (tm.tm_year + 1900) as u16,
        month: (tm.tm_mon + 1) as u8,
        day: tm.tm_mday as u8,
        weekday: monday_first(tm.tm_wday as u16),
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
        second: tm.tm_sec as u8,
        millis,
    }
}

/// Numbers weekdays 0 for Sunday to 6 for Saturday. The `LocalTime` contract uses
/// 0 for Monday to 6 for Sunday.
fn monday_first(day_of_week: u16) -> u8 {
    ((day_of_week + 6) % 7) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_now_fields_are_in_range() {
        let t = local_now();
        assert!((1..=12).contains(&t.month), "{t:?}");
        assert!((1..=31).contains(&t.day), "{t:?}");
        assert!(t.weekday <= 6, "{t:?}");
        assert!(t.hour <= 23 && t.minute <= 59 && t.second <= 59, "{t:?}");
        assert!(t.millis <= 999, "{t:?}");
    }

    #[test]
    fn weekday_is_monday_based() {
        assert_eq!(monday_first(0), 6);
        assert_eq!(monday_first(1), 0);
        assert_eq!(monday_first(6), 5);
    }
}
