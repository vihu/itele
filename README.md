# itele

[![CI](https://github.com/vihu/itele/actions/workflows/ci.yml/badge.svg)](https://github.com/vihu/itele/actions/workflows/ci.yml)
[![Release](https://github.com/vihu/itele/actions/workflows/release.yml/badge.svg)](https://github.com/vihu/itele/actions/workflows/release.yml)

A native IPTV player for Linux and macOS with Xtream Codes support. Sign in
to one provider or several, browse their channels, movies and series side
by side, read the guide, search it all, and watch, in one window that
starts fast and stays light.

## Design

- libmpv does all demuxing, decoding and audio, so every codec and container
  mpv plays works here, in hardware where the GPU supports it.
- Frames stay on the GPU from mpv to the window: DMA-BUF into Vulkan on
  Linux, IOSurface into Metal on macOS. The UI is [Slint] on wgpu.
- Several providers at once: one provider's groups or all of them
  together, every row tagged with its provider, with search, favorites and
  the guide always across every provider.
- The guide is stored locally (SQLite with full-text search), so scrolling
  and searching it never waits on the network.
- Posters are decoded and scaled off the UI thread, and only those near the
  screen are kept, so scrolling a 20,000-title catalog keeps memory flat.
- Passwords live in the system keychain (Secret Service on Linux, Keychain
  on macOS), never in a file or a log.

Status: 0.1.0, not released yet (changes in `CHANGELOG.md`). Targets Linux
(Wayland) and macOS. itele brings no channels of its own: it plays what
your provider offers.

## What it does

| Area      | Supported                                                                                                                        |
| --------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Home      | The channel you watched last, still playing; then what you left unfinished, your favorite channels on now, and the newest movies |
| Live TV   | Groups, channels with logos and what is on now and next, a preview of the selected channel, fast zapping                         |
| Favorites | One list across providers, each row tagged with its provider, in your order; channels, movies and series; `F` or a heart adds    |
| Guide     | A grid of channels against time from each provider's XMLTV, kept fresh in the background; catch-up replays ended programmes      |
| Movies    | Poster grids by group, a page per title with plot, cast and rating; series list their seasons and episodes                       |
| Resume    | Movies and episodes continue where they stopped, progress on posters, and the next episode plays when one ends                   |
| Search    | At the top of every screen, results as you type: channels, movies and series by name, programmes by title, across providers      |
| Player    | Pause and rewind within live, Go live, seek, audio and subtitle menus, an `i` overlay with stream details, fullscreen            |
| Settings  | Providers with names of your choosing and what each refresh is doing; refresh intervals, guide time shift, decoding, languages   |

## The app

Download it from [Releases](https://github.com/vihu/itele/releases):

- Linux (x86_64 and arm64), either of:
  - `itele-<version>-<arch>.flatpak`: run
    `flatpak install --user itele-<version>-<arch>.flatpak`, then start
    itele from the app menu.
  - `itele-<version>-<arch>.AppImage`: `chmod +x` it and run it. Needs
    glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 and later).
- macOS, Apple silicon and Intel: `itele-<version>-macos-universal.zip`.
  Unzip it and move `itele.app` to Applications. It is not notarized: allow
  the first launch in System Settings > Privacy & Security > Open Anyway,
  or run `xattr -dr com.apple.quarantine /Applications/itele.app`.

Both carry libmpv. The first run asks for a provider's server, username
and password; add more from the account chip at the foot of the sidebar.

Or run it from source. itele needs libmpv 0.37 or newer (Arch:
`pacman -S mpv`, Debian 13 and Ubuntu 24.04:
`apt install libmpv-dev libegl-dev libgbm-dev`, macOS: `brew install mpv`)
and a Rust toolchain:

```text
cargo run --release
```

Settings are kept in the platform config directory, the watch history and
favorites in its data directory, and lists, posters and the guide in its
cache directory.

Without an account, `tools/fake-provider.py` serves a fake provider with
groups, logos, a guide, catch-up, movies and series, playing local video
files as channels and titles:

```text
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

CI runs all of this on Linux and macOS for every push to `main` and every
pull request.

- Unit tests for the Xtream client (lenient parsing of real-world account,
  stream, movie and series JSON, URL building, catch-up times in the
  server's time zone), the XMLTV parser, the guide store (import, now and
  next, search), the watch history, favorites and their order, the channel
  catalog across providers, the settings, and the provider settings and
  cache.
- `cargo clippy -D warnings`, `cargo fmt --check`, a build on the minimum
  Rust version, and `cargo deny check licenses`, which keeps every Rust
  dependency compatible with the GPL.
- Each change is also run against the fake provider, end to end.

## License

GPL-3.0-or-later, see `LICENSE`. Built from source, itele links the
system's libmpv and FFmpeg at run time; the release builds carry them, as
`THIRD-PARTY-NOTICES.md` describes. The bundled font, Schibsted Grotesk,
is under the SIL Open Font License 1.1, with its licence in
`ui/fonts/OFL.txt`.

[Slint]: https://slint.dev
