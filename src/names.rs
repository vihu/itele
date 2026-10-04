//! Names as the screens draw and find them: the short name on a fallback
//! logo tile, a stable tile colour, a provider's hue, grouped digits, and
//! search ranking.

use slint::Color;

/// Longest short name drawn on a fallback logo tile.
const SHORT_NAME_MAX: usize = 8;
/// Longest country or package prefix skipped in channel names (`UK:`).
const PREFIX_MAX: usize = 4;
/// Fallback logo tile colours, from the approved mockups.
const TINTS: [u32; 9] = [
    0x1f5e3b, 0x2a3cc7, 0xb3261e, 0x0f6e8c, 0x3b3f4a, 0xc2185b, 0x5b6b2e, 0xd35400, 0x6b5ca5,
];

/// Provider tag hues, from the approved mockups and away from the accent.
const HUES: [u32; 6] = [0x4fb3ff, 0xc792ea, 0x6fd3a0, 0xf78fb3, 0x8fa8ff, 0x5fd4e0];

/// The first word of `name` that says something, upper-cased and cut to
/// fit a logo tile. Skips a provider's country prefix (`UK: Sky Sports`,
/// `FR | TF1`) and one-letter words.
pub fn short_name(name: &str) -> String {
    let name = match name.split_once([':', '|']) {
        Some((prefix, rest)) if prefix.trim().len() <= PREFIX_MAX && !rest.trim().is_empty() => {
            rest
        }
        _ => name,
    };
    let word = |w: &&str| w.chars().filter(|c| c.is_alphanumeric()).count();
    let mut words = name.split_whitespace().filter(|w| word(w) > 0);
    let first = words
        .clone()
        .find(|w| word(w) > 1)
        .or_else(|| words.next())
        .unwrap_or("");
    first
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(SHORT_NAME_MAX)
        .flat_map(char::to_uppercase)
        .collect()
}

/// A stable tile colour for `name`.
pub fn tint(name: &str) -> Color {
    // FNV-1a: stable across runs, unlike the std hasher.
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let rgb = TINTS[(hash % TINTS.len() as u64) as usize];
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The hue of the provider at `index` in sign-in order, for its tags; the
/// same everywhere it shows.
pub fn provider_hue(index: usize) -> Color {
    let rgb = HUES[index % HUES.len()];
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `1234567` as `1,234,567`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// How well `name` matches a search: `None` unless it contains every word
/// of `query` (both lowercased); then 0 when it starts with the query, 1
/// when a word starts with its first word, and 2 otherwise.
pub fn search_rank(name: &str, query: &str) -> Option<u8> {
    let mut words = query.split_whitespace();
    let first = words.next()?;
    if !query.split_whitespace().all(|w| name.contains(w)) {
        return None;
    }
    Some(if name.starts_with(query) {
        0
    } else if name.split_whitespace().any(|w| w.starts_with(first)) {
        1
    } else {
        2
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_fit_a_tile() {
        assert_eq!(short_name("Volt Sports 1"), "VOLT");
        assert_eq!(short_name("Ciné+ Club"), "CINÉ");
        assert_eq!(short_name("Supercalifragilistic"), "SUPERCAL");
        assert_eq!(short_name("UK: Sky Sports Main Event FHD"), "SKY");
        assert_eq!(short_name("FR | TF1 HD"), "TF1");
        assert_eq!(short_name("B Atlas Nature HD"), "ATLAS");
        assert_eq!(
            short_name("Ratio: 16:9"),
            "RATIO",
            "long prefix is part of the name"
        );
        assert_eq!(short_name("A"), "A");
        assert_eq!(short_name(""), "");
    }

    #[test]
    fn search_rank_orders_prefixes_then_word_starts() {
        assert_eq!(search_rank("sports extra", "sports"), Some(0));
        assert_eq!(search_rank("volt sports 1", "sports"), Some(1));
        assert_eq!(search_rank("esports arena", "sports"), Some(2));
        assert_eq!(search_rank("volt sports 1", "volt 2"), None);
        assert_eq!(search_rank("anything", "  "), None);
    }

    #[test]
    fn tint_is_stable() {
        assert_eq!(tint("Atlas Nature HD"), tint("Atlas Nature HD"));
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
