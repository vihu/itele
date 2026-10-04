# itele

A native IPTV player for Linux and macOS with Xtream Codes support. Sign in
to one provider or several, browse their channels side by side, read the
guide, search it, and watch, all in one window that starts fast and stays
light.

## Design

- libmpv does all demuxing, decoding and audio, so every codec and container
  mpv plays works here, in hardware where the GPU supports it.
- Frames stay on the GPU from mpv to the window: DMA-BUF into Vulkan on
  Linux, IOSurface into Metal on macOS. The UI is [Slint] on wgpu.
- Several providers at once from day one: one list per provider or all of
  them together, with search and the guide always across every provider.
- The guide is stored locally (SQLite with full-text search), so scrolling
  and searching it never waits on the network.
- Passwords live in the system keychain (Secret Service or KWallet on Linux,
  Keychain on macOS), never in a file or a log.

Status: 0.1.0, not released. Targets Linux (Wayland) and macOS. Live TV, the
guide, search and catch-up work; Home, movies, series and favourites are
next.

## What it does

- Live TV: groups on the left, channels with logos and what is on now and
  next, and a preview of the selected channel.
- Provider switcher: one provider's groups at a time, or "All providers"
  with every group tagged by its provider.
- TV guide: a grid of channels against time, 8 days back to 8 days ahead,
  refreshed every 12 hours from each provider's XMLTV.
- Search: channels by name and programmes by title, across all providers,
  with what is live and what can be replayed marked.
- Catch-up: Enter on an ended programme replays it from the provider's
  archive, when the channel has one.
- Player: pause and rewind within live, Go Live, seek, audio and subtitle
  tracks, and an `i` overlay with stream, video and audio details in the
  style of mpv's stats.

## Running it

itele needs libmpv (Arch: `pacman -S mpv`, Debian and Ubuntu:
`apt install libmpv-dev`, macOS: `brew install mpv`) and a Rust toolchain:

```bash
cargo run --release
```

The first run asks for a provider's server, username and password. Add more
from Settings in the rail. Settings are kept in the platform config
directory, lists and the guide in its cache directory.

Without an account, `tools/fake-provider.py` serves a fake provider with
groups, logos, a guide and catch-up, streaming local video files as live
channels:

```bash
python3 tools/fake-provider.py --media clip1.ts clip2.mp4
```

Then sign in to `http://127.0.0.1:8089` as `demo` / `demo`. Start a second
one with `--port 8090 --prefix "B "` to try several providers.

## Keys

| Where   | Keys                                                                  |
| ------- | --------------------------------------------------------------------- |
| Live TV | `↑` `↓` channel, `←` `→` group, `Enter` watch, `P` next provider      |
| Live TV | `G` guide, `/` or `Ctrl+K` search                                     |
| Guide   | arrows move, `PgUp` `PgDn` page, `N` now, `Enter` watch, `Esc` back   |
| Search  | type to search, `↑` `↓` choose, `Enter` watch or replay, `Esc` back   |
| Player  | `Space` pause, `←` `→` seek 10 s, `L` go live, `↑` `↓` change channel |
| Player  | `M` mute, `+` `-` volume, `A` audio, `S` subtitles, `F` fullscreen    |
| Player  | `I` stream info, `Esc` or `Backspace` back                            |

## How it is checked

- Unit tests for the Xtream client (lenient parsing of real-world account
  and stream JSON, URL building, catch-up times in the server's time zone),
  the XMLTV parser, the guide store (import, now and next, search) and the
  provider settings and cache.
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
