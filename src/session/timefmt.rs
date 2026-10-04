//! Times as the screens show them: local clock times, day labels, and
//! spans like "Starts in 38 min".

use itele::epg::Programme;

/// Ruler and guide window steps, in seconds.
pub(super) const HALF_HOUR: i64 = 1800;

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

/// Unix seconds as local `21:00`.
pub(super) fn clock(at: i64) -> String {
    jiff::Timestamp::from_second(at).map_or_else(
        |_| String::new(),
        |t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%H:%M")
                .to_string()
        },
    )
}

/// `38 min left`, `1 h 12 min left`.
pub(super) fn minutes_left(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 59) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min left"),
        (h, 0) => format!("{h} h left"),
        (h, m) => format!("{h} h {m} min left"),
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
