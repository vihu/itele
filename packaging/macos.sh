#!/usr/bin/env bash
# Builds itele.app for macOS, in two steps:
#
#   macos.sh build
#     On a Mac of either architecture, with Homebrew's mpv, dylibbundler
#     and librsvg (brew install mpv dylibbundler librsvg): itele.app for
#     this architecture, with libmpv and the libraries it pulls in copied
#     into Contents/Frameworks, as target/dist/itele-<arch>.app.tar.
#
#   macos.sh merge <arm64.app.tar> <x86_64.app.tar>
#     Joins the two into one app for Apple silicon and Intel, ad-hoc
#     signed, as target/dist/itele-<version>-macos-universal.zip. Both
#     builds must carry the same libraries, which they do when Homebrew is
#     current on both.
#
# Ad-hoc signing seals the bundle: a download then gets macOS's "could not
# verify" prompt, which Privacy & Security > Open Anyway clears, instead
# of being reported as damaged. The oldest macOS it runs on is the newest
# any bundled library was built for; Info.plist says which.
set -euo pipefail
cd "$(dirname "$0")/.."

id=io.github.vihu.itele
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml)

build() {
  local arch work app iconset size
  arch=$(uname -m)
  export MACOSX_DEPLOYMENT_TARGET=11.0
  cargo build --profile dist --locked

  work=$(mktemp -d)
  app=$work/itele.app
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
  cp target/dist/itele "$app/Contents/MacOS/itele"
  dylibbundler -od -b -cd \
    -x "$app/Contents/MacOS/itele" \
    -d "$app/Contents/Frameworks/" \
    -p @executable_path/../Frameworks/ \
    -s "$(brew --prefix)/lib"

  iconset=$work/itele.iconset
  mkdir "$iconset"
  for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" "packaging/$id.svg" \
      -o "$iconset/icon_${size}x${size}.png"
    rsvg-convert -w "$((size * 2))" -h "$((size * 2))" "packaging/$id.svg" \
      -o "$iconset/icon_${size}x${size}@2x.png"
  done
  iconutil -c icns -o "$app/Contents/Resources/itele.icns" "$iconset"

  mkdir -p target/dist
  tar -C "$work" -cf "target/dist/itele-$arch.app.tar" itele.app
}

merge() {
  local work arm intel app file minos
  work=$(mktemp -d)
  mkdir "$work/arm64" "$work/x86_64"
  tar -C "$work/arm64" -xf "$1"
  tar -C "$work/x86_64" -xf "$2"
  arm=$work/arm64/itele.app
  intel=$work/x86_64/itele.app
  app=$work/itele.app
  cp -R "$arm" "$app"

  if ! diff <(cd "$arm" && find . -type f | sort) <(cd "$intel" && find . -type f | sort); then
    echo "the arm64 and x86_64 builds carry different files" >&2
    exit 1
  fi
  while IFS= read -r file; do
    if file "$arm/$file" | grep -q 'Mach-O'; then
      lipo -create -output "$app/$file" "$arm/$file" "$intel/$file"
    fi
  done < <(cd "$arm" && find . -type f)

  # The newest macOS any part was built for.
  minos=$(find "$app/Contents" -type f -exec sh -c 'file "$1" | grep -q Mach-O && otool -l "$1"' _ {} \; |
    awk '/minos/ {print $2}' | sort -V | tail -n 1)

  cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>itele</string>
  <key>CFBundleExecutable</key><string>itele</string>
  <key>CFBundleIconFile</key><string>itele</string>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>itele</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.entertainment</string>
  <key>LSMinimumSystemVersion</key><string>$minos</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
  plutil -lint "$app/Contents/Info.plist"

  # Libraries first, then the app that loads them.
  find "$app/Contents/Frameworks" -type f -exec codesign --force --sign - {} \;
  codesign --force --sign - "$app"
  codesign --verify --strict --deep "$app"

  mkdir -p target/dist
  ditto -c -k --keepParent "$app" "target/dist/itele-$version-macos-universal.zip"
  echo "itele.app runs on macOS $minos or newer"
}

case ${1:-} in
build) build ;;
merge) merge "$2" "$3" ;;
*)
  echo "usage: macos.sh build | macos.sh merge <arm64.app.tar> <x86_64.app.tar>" >&2
  exit 1
  ;;
esac
