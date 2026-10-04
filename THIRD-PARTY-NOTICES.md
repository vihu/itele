# Third-party notices

## Schibsted Grotesk

`ui/fonts/SchibstedGrotesk.ttf` is the Schibsted Grotesk typeface by
Schibsted Media, under the SIL Open Font License 1.1; the licence is in
`ui/fonts/OFL.txt`.

## Libraries in the release builds

Built from source, itele links the system's libmpv at run time and
carries none of it. The release builds carry libmpv and the libraries it
loads, unmodified apart from how they were configured:

- mpv (libmpv), GPL-2.0-or-later, https://mpv.io
- FFmpeg, GPL-3.0-or-later as these builds configure it,
  https://ffmpeg.org
- libplacebo, LGPL-2.1-or-later, https://code.videolan.org/videolan/libplacebo
- libass, ISC, https://github.com/libass/libass
- and what those link to in turn, each under its own licence.

Where each build takes them from, and so where their source is:

- Flatpak: built from the upstream releases that
  `packaging/io.github.vihu.itele.yml` pins (mpv 0.41.0, FFmpeg 9.0.2,
  libplacebo 7.360.1, libass 0.17.5, with the options listed there), on the
  Freedesktop 26.08 runtime.
- AppImage: Ubuntu 24.04's packages (`libmpv2` and what it depends on);
  `apt-get source` or https://packages.ubuntu.com gives each one's source.
- macOS: Homebrew's `mpv` and what it depends on; each formula names its
  source archive (`brew info --json=v2 mpv`).

The licence texts of the GPL and LGPL are at
https://www.gnu.org/licenses/.
