//! Times as the screens show them: local clock times, day labels, and
//! spans like "Starts in 38 min".

use std::sync::atomic::{AtomicBool, Ordering};

use itele::epg::Programme;

/// Ruler and guide window steps, in seconds.
pub(super) const HALF_HOUR: i64 = 1800;

/// Clock times in 12-hour form (`9:30 PM`), as the user set; read by every
/// formatter here, so it is one flag rather than an argument everywhere.
static TWELVE_HOUR: AtomicBool = AtomicBool::new(false);

/// Writes clock times in 12-hour form from now on, or in 24-hour form.
pub(super) fn set_twelve_hour(twelve: bool) {
    TWELVE_HOUR.store(twelve, Ordering::Relaxed);
}

pub(super) fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// `at` rounded down to the half hour.
pub(super) fn half_hour(at: i64) -> i64 {
    at.div_euclid(HALF_HOUR) * HALF_HOUR
}

fn zoned(at: i64) -> Option<jiff::Zoned> {
    jiff::Timestamp::from_second(at)
        .ok()
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()))
}

/// Days from `now` to `at`, in the local calendar.
fn days_from(at: i64, now: i64) -> Option<i64> {
    let (at, now) = (zoned(at)?.date(), zoned(now)?.date());
    Some(i64::from(now.until(at).ok()?.get_days()))
}

/// `Today, Sat 4 Oct`, `Tomorrow, Sun 5 Oct`, `Mon 6 Oct`.
pub(super) fn day_label(at: i64, now: i64) -> String {
    let date = zoned(at)
        .map(|z| z.strftime("%a %-d %b").to_string())
        .unwrap_or_default();
    match days_from(at, now) {
        Some(0) => format!("Today, {date}"),
        Some(1) => format!("Tomorrow, {date}"),
        Some(-1) => format!("Yesterday, {date}"),
        _ => date,
    }
}

/// `Today 21:00`, `Tomorrow 06:00`, `Mon 6 Oct 21:00`.
pub(super) fn day_time(at: i64, now: i64) -> String {
    let day = match days_from(at, now) {
        Some(0) => "Today".to_owned(),
        Some(1) => "Tomorrow".to_owned(),
        Some(-1) => "Yesterday".to_owned(),
        _ => zoned(at)
            .map(|z| z.strftime("%a %-d %b").to_string())
            .unwrap_or_default(),
    };
    format!("{day} {}", clock(at))
}

/// `38 min left`, `Starts in 2 h 10 min`, `Ended 25 min ago`.
pub(super) fn when(programme: &Programme, now: i64) -> String {
    let span = |seconds: i64| {
        let minutes = (seconds.max(0) + 59) / 60;
        match (minutes / 60, minutes % 60) {
            (0, m) => format!("{m} min"),
            (h, 0) => format!("{h} h"),
            (h, m) => format!("{h} h {m} min"),
        }
    };
    if programme.stop <= now {
        format!("Ended {} ago", span(now - programme.stop))
    } else if programme.start <= now {
        format!("{} left", span(programme.stop - now))
    } else {
        format!("Starts in {}", span(programme.start - now))
    }
}

/// Unix seconds as local `21:00`, or `9:00 PM` in 12-hour form.
pub(super) fn clock(at: i64) -> String {
    let format = if TWELVE_HOUR.load(Ordering::Relaxed) {
        "%-I:%M %p"
    } else {
        "%H:%M"
    };
    jiff::Timestamp::from_second(at).map_or_else(
        |_| String::new(),
        |t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime(format)
                .to_string()
        },
    )
}

/// `38 min left`, `1 h 12 min left`, rounded up.
pub(super) fn minutes_left(seconds: i64) -> String {
    format!("{} left", hours_minutes((seconds.max(0) + 59) / 60))
}

/// How long ago something `seconds` old happened: `just now`,
/// `14 min ago`, `2 h ago`, `yesterday`, `3 days ago`.
pub(super) fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => "just now".to_owned(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 2 * 86_400 => "yesterday".to_owned(),
        s => format!("{} days ago", s / 86_400),
    }
}

/// A running time: `45 min`, `1 h 34 min`, to the nearest minute.
pub(super) fn runtime(seconds: i64) -> String {
    hours_minutes((seconds.max(0) + 30) / 60)
}

fn hours_minutes(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(start: i64, stop: i64) -> Programme {
        Programme {
            start,
            stop,
            title: String::new(),
            description: String::new(),
        }
    }

    #[test]
    fn minutes_left_rounds_up_and_uses_hours() {
        assert_eq!(minutes_left(38 * 60 - 20), "38 min left");
        assert_eq!(minutes_left(3600), "1 h left");
        assert_eq!(minutes_left(72 * 60), "1 h 12 min left");
        assert_eq!(minutes_left(-5), "0 min left");
        assert_eq!(runtime(5645), "1 h 34 min");
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(14 * 60 + 5), "14 min ago");
        assert_eq!(ago(30 * 3600), "yesterday");
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(runtime(2 * 3600 + 10), "2 h");
    }

    #[test]
    fn when_reads_naturally() {
        let show = p(1000, 4600);
        assert_eq!(when(&show, 1000 - 30 * 60), "Starts in 30 min");
        assert_eq!(when(&show, 1000 + 22 * 60), "38 min left");
        assert_eq!(when(&show, 4600 + 2 * 3600), "Ended 2 h ago");
    }

    #[test]
    fn half_hour_rounds_down() {
        assert_eq!(half_hour(1799), 0);
        assert_eq!(half_hour(1800), 1800);
        assert_eq!(half_hour(-1), -1800);
    }
}
