//! The names and status lines the screens show for providers: the status
//! under "Live TV", channel counts, expiry dates, hosts and initials.

use slint::SharedString;

use super::Session;
use crate::live::View;
use crate::names::thousands;
use itele::provider::Library;

impl Session {
    /// The status line under "Live TV": the viewed provider's, or a summary
    /// of all of them.
    pub(super) fn view_status(&self) -> SharedString {
        let state = self.state.borrow();
        match state.catalog.view() {
            View::One(id) => state
                .slots
                .iter()
                .find(|slot| &slot.provider.id == id)
                .map_or_else(SharedString::new, |slot| {
                    format!("{} · {}", slot.status, slot.provider.name).into()
                }),
            View::All => match state.slots.len() {
                0 => SharedString::new(),
                1 => state.slots[0].status.as_str().into(),
                n => format!(
                    "{n} providers · {} channels",
                    thousands(state.catalog.total_channels())
                )
                .into(),
            },
        }
    }
}

/// For example `1,284 channels`.
pub(super) fn library_status(library: &Library) -> String {
    format!("{} channels", thousands(library.streams.len()))
}

/// When the account expires, for example `12 Jan 2027`; `None` when it
/// does not.
pub(super) fn expiry(account: &itele::xtream::Account) -> Option<String> {
    let secs = i64::try_from(account.expires_at?).ok()?;
    let at = jiff::Timestamp::from_second(secs).ok()?;
    Some(
        at.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%-d %b %Y")
            .to_string(),
    )
}

/// `http://tv.example.com:8080` as `tv.example.com`.
pub(super) fn host_of(server: &str) -> &str {
    let rest = server.split_once("://").map_or(server, |(_, rest)| rest);
    rest.split([':', '/']).next().unwrap_or(rest)
}

/// Two letters for the avatar, from the username.
pub(super) fn initials(username: &str) -> String {
    username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_two_letters() {
        assert_eq!(initials("north.wind"), "NO");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn host_of_strips_scheme_port_and_path() {
        assert_eq!(host_of("http://tv.example.com:8080"), "tv.example.com");
        assert_eq!(host_of("https://tv.example.com/iptv"), "tv.example.com");
        assert_eq!(host_of("tv.example.com"), "tv.example.com");
    }
}
