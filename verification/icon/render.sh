#!/usr/bin/env bash
# Render the app icon at 16/32/64/256 px and the symbolic icon on light and
# dark backgrounds into verification/icon/ for visual review. Needs
# rsvg-convert and ImageMagick (magick). Exit 0 when all renders were written.
set -euo pipefail
cd -- "$(dirname -- "$0")/../.."
out=verification/icon; t="$(mktemp -d)"; trap 'rm -rf -- "$t"' EXIT
for s in 16 32 64 256; do rsvg-convert -w $s -h $s resources/dev.nilltadios.GearLeverCosmic.svg -o "$t/i$s.png"; done
for s in 16 32 64; do rsvg-convert -w $s -h $s resources/dev.nilltadios.GearLeverCosmic-symbolic.svg -o "$t/s$s.png"; done
magick -size 700x300 xc:'#f2f2f2' \( -size 350x300 xc:'#1b1b1f' \) -geometry +350+0 -composite \
  "$t/i256.png" -geometry +20+22 -composite "$t/i64.png" -geometry +290+30 -composite "$t/i32.png" -geometry +290+110 -composite "$t/i16.png" -geometry +290+160 -composite \
  "$t/i16.png" -geometry +380+40 -composite "$t/i32.png" -geometry +380+80 -composite "$t/i64.png" -geometry +380+130 -composite "$t/i256.png" -geometry +450+22 -composite \
  "$out/app-icon-light-dark-16-32-64-256.png"
magick -size 260x90 xc:'#f2f2f2' \( -size 130x90 xc:'#1b1b1f' \) -geometry +130+0 -composite \
  "$t/s64.png" -geometry +10+13 -composite "$t/s32.png" -geometry +80+20 -composite "$t/s16.png" -geometry +85+60 -composite \
  "$t/s64.png" -geometry +140+13 -composite "$t/s32.png" -geometry +210+20 -composite "$out/symbolic-16-32-64.png"
rsvg-convert -w 512 -h 512 resources/dev.nilltadios.GearLeverCosmic.svg -o "$t/512.png"
cmp -s "$t/512.png" resources/dev.nilltadios.GearLeverCosmic.png && echo "PNG export matches the SVG" || echo "PNG export differs from the SVG: re-export it"
echo "Wrote $out/app-icon-light-dark-16-32-64-256.png and $out/symbolic-16-32-64.png"
