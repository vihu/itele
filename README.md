# itele

A native IPTV player for Linux and macOS with Xtream Codes support. Sign in
to one provider or several, browse their channels, movies and series side
by side, read the guide, search it all, and watch, in one window that
starts fast and stays light.

## Design

- libmpv does all demuxing, decoding and audio, so every codec and container
  mpv plays works here, in hardware where the GPU supports it.
- Frames stay on the GPU from mpv to the window: DMA-BUF into Vulkan on
  Linux, IOSurface into Metal on macOS. The UI is [Slint] on wgpu.
- Several providers at once from day one: one list per provider or all of
  them together, with search and the guide always across every provider.
- The guide is stored locally (SQLite with full-text search), so scrolling
  and searching it never waits on the network.
- Posters are decoded and scaled off the UI thread, and only those near the
  screen are kept, so scrolling a 20,000-title catalog keeps memory flat.
- Passwords live in the system keychain (Secret Service or KWallet on Linux,
  Keychain on macOS), never in a file or a log.

Status: 0.1.0, not released. Targets Linux (Wayland) and macOS. Home, Live
TV, favorites, the guide, search, catch-up, movies, series and settings
work.

## What it does

- Home: the channel you watched last, still playing, then what you left
  unfinished, your favorite channels with what is on, and the newest
  movies.
- Live TV: groups on the left, channels with logos and what is on now and
  next, and a preview of the selected channel.
- Favorites: one list across providers, every channel tagged with its
  provider, in the order you set; `F` or a heart adds a channel, movie or
  series from anywhere. Zapping in the player follows the list when you
  watch from it, and a channel the provider drops stays, dimmed.
- Providers: the account chip at the foot of the sidebar switches between
  one provider's groups and "All providers", with every group tagged by its
  provider, and holds Add provider and Settings.
- Search at the top of every screen, with results as you type.
- TV guide: a grid of channels against time, from each provider's XMLTV,
  kept fresh in the background while itele runs.
- Movies and Series: poster grids with the same groups, and a page per
  title with plot, cast and rating; a series' page lists its seasons and
  episodes.
- Resume: movies and episodes continue where they stopped, progress shows
  on posters and episodes, and the next episode plays when one ends.
- Search: channels, movies and series by name and programmes by title,
  across all providers, with what is live and what can be replayed marked.
- Catch-up: Enter on an ended programme replays it from the provider's
  archive, when the channel has one.
- Player: pause and rewind within live, Go Live, seek, menus for the audio
  and subtitle tracks, and an `i` overlay with stream, video and audio
  details in the style of mpv's stats.
- Settings: providers with names of your choosing and what each refresh is
  doing, how often guides and lists refresh, a time shift for guides that
  are off, hardware decoding, preferred audio and subtitle languages, the
  live stream format and rewind buffer, the sidebar, the start screen and
  the clock; About names the version and the commit it was built from.

## Running it

itele needs libmpv (Arch: `pacman -S mpv`, Debian and Ubuntu:
`apt install libmpv-dev`, macOS: `brew install mpv`) and a Rust toolchain:

```bash
cargo run --release
```

The first run asks for a provider's server, username and password. Add more
from the account chip at the foot of the sidebar. Settings are kept in the
platform config directory, the watch history and favorites in its data
directory, and lists, posters and the guide in its cache directory.

Without an account, `tools/fake-provider.py` serves a fake provider with
groups, logos, a guide, catch-up, movies and series, playing local video
files as channels and titles:

```bash
python3 tools/fake-provider.py --media clip1.ts clip2.mp4
```

Then sign in to `http://127.0.0.1:8089` as `demo` / `demo`. Start a second
one with `--port 8090 --prefix "B "` to try several providers;
`--movies 20000` tries a large catalog and `--posters DIR` uses your own
images as posters.

## Keys

| Where     | Keys                                                                     |
| --------- | ------------------------------------------------------------------------ |
| Anywhere  | `/` or `Ctrl+K` search, `Ctrl+B` sidebar, `Ctrl+,` Settings              |
| Search    | type in the top bar, `↑` `↓` choose, `Enter` watch or open, `Esc` back   |
| Search    | with nothing chosen, `Enter` shows all results                           |
| Home      | arrows move between the rows and along them, `Enter` watch or open       |
| Live TV   | `↑` `↓` channel, `←` `→` group, `Enter` watch, `F` favorite              |
| Live TV   | `G` guide, `P` next provider                                             |
| Favorites | `↑` `↓` choose, `Alt+↑` `Alt+↓` move, `F` or `Delete` remove             |
| Favorites | `Tab` channels, movies, series; `Enter` watch or open                    |
| Guide     | arrows move, `PgUp` `PgDn` page, `N` now, `F` favorite, `Enter` watch    |
| Movies    | arrows move, `←` from the first column to the groups, `Enter` open       |
| Page      | `Enter` play or resume, `B` from the beginning, `F` favorite, `Esc` back |
| Page      | on a series: `↑` `↓` episode, `←` `→` season                             |
| Player    | `Space` pause, `←` `→` seek 10 s, `L` go live, `↑` `↓` change channel    |
| Player    | `M` mute, `+` `-` volume, `A` audio menu, `S` subtitles menu             |
| Player    | `F` fullscreen; in a menu `↑` `↓` choose, `Enter` pick, `Esc` close      |
| Player    | `H` favorite, `I` stream info, `Esc` or `Backspace` back                 |

Series work like Movies. On a movie or an episode the player has no Go live
or channel keys. Watching from Favorites, `↑` `↓` in the player follow your
list across providers. In the guide, a click on a channel's name plays it
in the preview above.

## How it is checked

- Unit tests for the Xtream client (lenient parsing of real-world account,
  stream, movie and series JSON, URL building, catch-up times in the
  server's time zone), the XMLTV parser, the guide store (import, now and
  next, search), the watch history, favorites and their order, the
  channel catalog across providers, the settings, and the provider
  settings and cache.
- `cargo deny check licenses` keeps every Rust dependency compatible with
  the GPL.
- Each change is also run against the fake provider, end to end.

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check licenses
```

## Licence

GPL-3.0-or-later, see `LICENSE`. libmpv and FFmpeg are linked at run time
from the system. The bundled font, Schibsted Grotesk, is under the SIL Open
Font License 1.1, with its licence in `ui/fonts/OFL.txt`.

[Slint]: https://slint.dev
