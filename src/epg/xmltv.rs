//! XMLTV: programme times and the streaming `<programme>` parser.

use std::io::BufRead;
use std::ops::Range;

use quick_xml::events::{BytesStart, Event};

use super::{Error, Programme, Result};

/// Parses an XMLTV time: `20261004213000 +0100`, with or without the
/// offset (UTC then) or the seconds.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let rest = text[digits.len()..].trim();
    let number = |range: Range<usize>| digits.get(range)?.parse::<i32>().ok();
    let (year, month, day) = (number(0..4)?, number(4..6)?, number(6..8)?);
    let (hour, minute) = (number(8..10)?, number(10..12)?);
    let second = if digits.len() >= 14 {
        number(12..14)?
    } else {
        0
    };
    let offset_seconds = match rest.as_bytes() {
        [] => 0,
        [sign @ (b'+' | b'-'), ..] if rest.len() >= 5 => {
            let hours: i32 = rest.get(1..3)?.parse().ok()?;
            let minutes: i32 = rest.get(3..5)?.parse().ok()?;
            let seconds = hours * 3600 + minutes * 60;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ if rest.eq_ignore_ascii_case("z") || rest.eq_ignore_ascii_case("utc") => 0,
        _ => return None,
    };
    let civil = jiff::civil::DateTime::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        0,
    )
    .ok()?;
    let offset = jiff::tz::Offset::from_seconds(offset_seconds).ok()?;
    Some(offset.to_timestamp(civil).ok()?.as_second())
}

/// Which text the parser is collecting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    None,
    Title,
    Description,
}

/// Streams the `<programme>` elements of `xml` into `found` with their
/// lowercased channel id. Programmes with unreadable times are skipped.
pub(super) fn parse(
    xml: impl BufRead,
    mut found: impl FnMut(String, Programme) -> Result,
) -> Result {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut current: Option<(String, Programme)> = None;
    let mut field = Field::None;
    let mut has_title = false;
    loop {
        let event = reader.read_event_into(&mut buf).map_err(Error::Xml)?;
        match event {
            Event::Start(e) if e.local_name().as_ref() == "programme" => {
                current = start_programme(&e);
                has_title = false;
            }
            Event::Start(e) if current.is_some() => {
                field = match e.local_name().as_ref() {
                    // The first title is the main one; others are translations.
                    "title" if !has_title => Field::Title,
                    "desc" => Field::Description,
                    _ => Field::None,
                };
            }
            Event::Text(t) => push(&mut current, field, &t.xml10_content()),
            Event::CData(c) => push(&mut current, field, &c),
            Event::GeneralRef(r) => {
                let text = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => quick_xml::escape::resolve_predefined_entity(r.as_ref())
                        .map_or_else(|| format!("&{};", r.as_ref()), str::to_owned),
                };
                push(&mut current, field, &text);
            }
            Event::End(e) => match e.local_name().as_ref() {
                "programme" => {
                    if let Some((channel, mut programme)) = current.take() {
                        programme.title = programme.title.trim().to_owned();
                        programme.description = programme.description.trim().to_owned();
                        if !programme.title.is_empty() {
                            found(channel, programme)?;
                        }
                    }
                }
                "title" if field == Field::Title => {
                    has_title = true;
                    field = Field::None;
                }
                _ => field = Field::None,
            },
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

/// A programme from its start tag, or `None` when it lacks a channel or a
/// readable start and stop.
fn start_programme(e: &BytesStart) -> Option<(String, Programme)> {
    let attribute = |name: &str| {
        e.try_get_attribute(name).ok().flatten().and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
    };
    let channel = attribute("channel")?.trim().to_lowercase();
    let start = parse_time(&attribute("start")?)?;
    let stop = parse_time(&attribute("stop")?)?;
    Some((
        channel,
        Programme {
            start,
            stop,
            title: String::new(),
            description: String::new(),
        },
    ))
}

fn push(current: &mut Option<(String, Programme)>, field: Field, text: &str) {
    if let Some((_, programme)) = current {
        match field {
            Field::Title => programme.title.push_str(text),
            Field::Description => programme.description.push_str(text),
            Field::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-04 21:22 UTC.
    const NOW: i64 = 1_791_148_920;

    #[test]
    fn parse_time_reads_offsets_and_short_forms() {
        assert_eq!(parse_time("20261004212200 +0000"), Some(NOW));
        assert_eq!(parse_time("20261004222200 +0100"), Some(NOW));
        assert_eq!(parse_time("20261004162200 -0500"), Some(NOW));
        assert_eq!(parse_time("20261004212200"), Some(NOW), "no offset is UTC");
        assert_eq!(parse_time("202610042122"), Some(NOW), "no seconds");
        assert_eq!(parse_time("20261004212200 Z"), Some(NOW));
        assert_eq!(parse_time("garbage"), None);
        assert_eq!(parse_time("20261304212200 +0000"), None, "month 13");
        assert_eq!(parse_time("20261004212200 +01"), None);
    }
}
