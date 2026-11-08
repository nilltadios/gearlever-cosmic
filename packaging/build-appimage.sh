#!/usr/bin/env bash
# Build the AppImage the Anylinux way (pkgforge-dev/Anylinux-AppImages):
# quick-sharun bundles the binaries with every library they need, including
# glibc, into an AppDir, and packs it with uruntime (DwarFS, no FUSE 2). The
# result runs on any Linux distribution, independent of the build system.
#
# Usage: packaging/build-appimage.sh [BIN_DIR] [OUT_DIR]
#   BIN_DIR  folder with release gearlever-cosmic and gearlever-cosmic-cli (default target/release)
#   OUT_DIR  output folder (default dist): GearLever_COSMIC-VERSION-x86_64.AppImage, .zsync, .sha256
# quick-sharun is pinned to a commit and checked by SHA-256; it pins and
# checks the sharun, appimagetool (uruntime) and onelf it downloads.
# To move to the newest quick-sharun, update QS_COMMIT and QS_SHA (see
# docs/development.md, "Updating the AppImage tooling").
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
QS_COMMIT=1a5594c7f47ec5cb46ce70f6e11b5c5781f17f5f
QS_SHA=35ba7b95f91f4665707c4cab5ed99407ceea636c15675990bb42816d6d84cc33
bin="$(realpath "${1:-target/release}")"
out="$(realpath -m "${2:-dist}")"
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["package"]["version"])')
app=dev.nilltadios.GearLeverCosmic
cache="$PWD/packaging/cache"
mkdir -p "$cache" "$out"
qs="$cache/quick-sharun-$QS_COMMIT.sh"
if [[ ! -f $qs ]] || ! echo "$QS_SHA  $qs" | sha256sum -c --status; then
  curl -fsSL -o "$qs.part" "https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages/$QS_COMMIT/useful-tools/quick-sharun.sh"
  echo "$QS_SHA  $qs.part" | sha256sum -c --status || { echo "quick-sharun checksum mismatch" >&2; rm -f "$qs.part"; exit 1; }
  mv "$qs.part" "$qs"
fi
chmod +x "$qs"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
sed "s/^X-AppImage-Version=.*/X-AppImage-Version=$version/" resources/$app.desktop > "$work/$app.desktop"
repo="${GITHUB_REPOSITORY:-nilltadios/gearlever-cosmic}"
export APPDIR="$work/AppDir"
export DESKTOP="$work/$app.desktop"
export ICON="$PWD/resources/$app.svg"
export OUTPATH="$out"
export OUTNAME="GearLever_COSMIC-$version-x86_64.AppImage"
export UPINFO="${UPINFO:-gh-releases-zsync|${repo%%/*}|${repo#*/}|latest|GearLever_COSMIC-*-x86_64.AppImage.zsync}"
export MAIN_BIN=gearlever-cosmic
# appimagetool writes X-AppImage-Version from VERSION ("UNKNOWN" if unset).
export VERSION="$version"
# Rendering is wgpu (Vulkan, then OpenGL) with a software fallback. Mesa is
# not bundled: the host's GPU drivers are loaded through cross-libc-dlopen,
# which keeps the AppImage small (a bundled Mesa with LLVM was 340 MB).
export DEPLOY_VULKAN=0 DEPLOY_OPENGL=0
# No strace run: it would also pick up the GPU drivers (Mesa + LLVM) the
# test launch happened to load.
export STRACE_MODE=0
# If quick-sharun starts the app for any check, it must never act on the
# builder's real files: isolated mode and a throwaway home.
export GEARLEVER_COSMIC_UI_ISOLATED=1
qs_home="$work/home"; mkdir -p "$qs_home"
export TMPDIR="$work"
# The font library mentions /usr/share/fonts, which makes quick-sharun copy
# every system font (670 MB here). The app reads the host's fonts, so an
# existing (empty) folder tells quick-sharun to skip the copy.
mkdir -p "$APPDIR/share/fonts"
( cd "$work" && HOME="$qs_home" XDG_CONFIG_HOME="$qs_home/.config" XDG_DATA_HOME="$qs_home/.local/share" "$qs" "$bin/gearlever-cosmic" "$bin/gearlever-cosmic-cli" )
# quick-sharun rewrites /usr/lib and /usr/share strings in the binaries to
# point into the AppImage. This app must see the host's folders (library
# check) and read /usr/share/... inside other AppImages, so the unmodified
# binaries go back; bundled libraries are still found through sharun.
for b in gearlever-cosmic gearlever-cosmic-cli; do
  install -m755 "$bin/$b" "$APPDIR/shared/bin/$b"
done
rm -f "$APPDIR/bin/01-path-mapping-hardcoded.hook"
rmdir "$APPDIR/share/fonts" 2>/dev/null || true
if grep -aq "/tmp/[A-Za-z0-9]\{6,\}/lib\b" "$APPDIR/shared/bin/gearlever-cosmic-cli"; then
  echo "A patched binary is still in the AppDir" >&2; exit 1
fi
# App data: the private fallback icon theme, AppStream data, docs.
mkdir -p "$APPDIR/share/icons" "$APPDIR/share/metainfo" "$APPDIR/share/gearlever-cosmic"
cp -r resources/icons/GearLeverCosmic "$APPDIR/share/icons/"
install -Dm644 resources/$app.png "$APPDIR/share/icons/hicolor/512x512/apps/$app.png"
install -Dm644 resources/$app.metainfo.xml "$APPDIR/share/metainfo/$app.metainfo.xml"
cp -r docs llms.txt LICENSE NOTICE "$APPDIR/share/gearlever-cosmic/"
install -Dm755 packaging/cli.hook "$APPDIR/bin/50-gearlever-cli.hook"
rm -f "$out/$OUTNAME" "$out/$OUTNAME.zsync"
( cd "$work" && HOME="$qs_home" XDG_CONFIG_HOME="$qs_home/.config" XDG_DATA_HOME="$qs_home/.local/share" "$qs" --make-appimage )
sha256sum "$out/$OUTNAME" > "$out/$OUTNAME.sha256"
echo "$out/$OUTNAME"
