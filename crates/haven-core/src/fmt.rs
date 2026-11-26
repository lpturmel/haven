//! Display formatting helpers.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local};

/// Finder-style decimal sizes: "0 bytes", "912 bytes", "4 KB", "12.3 MB".
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return if bytes == 1 {
            "1 byte".into()
        } else {
            format!("{bytes} bytes")
        };
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 || unit == 0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// "Today at 14:03", "Yesterday at 09:12", "Mar 4 at 10:00", "Mar 4, 2023".
pub fn date(time: Option<SystemTime>) -> String {
    let Some(time) = time else { return "--".into() };
    let dt: DateTime<Local> = time.into();
    let now = Local::now();
    let days = (now.date_naive() - dt.date_naive()).num_days();
    match days {
        0 => format!("Today at {}", dt.format("%H:%M")),
        1 => format!("Yesterday at {}", dt.format("%H:%M")),
        _ if dt.year() == now.year() => dt.format("%b %-d at %H:%M").to_string(),
        _ => dt.format("%b %-d, %Y").to_string(),
    }
}

/// Short relative time: "just now", "5 min ago", "3 h ago", "2 d ago", else a date.
pub fn relative(time: SystemTime) -> String {
    let Ok(age) = SystemTime::now().duration_since(time) else {
        return "just now".into();
    };
    let secs = age.as_secs();
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=604_799 => format!("{} d ago", secs / 86_400),
        _ => date(Some(time)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 bytes");
        assert_eq!(size(1), "1 byte");
        assert_eq!(size(999), "999 bytes");
        assert_eq!(size(1000), "1 KB");
        assert_eq!(size(1_234_567), "1.2 MB");
        assert_eq!(size(330_000_000_000), "330 GB");
        assert_eq!(size(999_999), "1.0 MB");
    }

    #[test]
    fn relative_times() {
        let now = SystemTime::now();
        assert_eq!(relative(now), "just now");
        assert_eq!(relative(now - Duration::from_secs(300)), "5 min ago");
        assert_eq!(relative(now - Duration::from_secs(7200)), "2 h ago");
        assert!(date(Some(now)).starts_with("Today at"));
        assert_eq!(date(None), "--");
    }
}
