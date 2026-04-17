#!/usr/bin/env bash
# Regenerates the macOS AppIcon.icns from plugins/oliv4600-pack/icon.svg
# Requires macOS built-ins: qlmanage, sips, iconutil.
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SVG="$ROOT/plugins/oliv4600-pack/icon.svg"
ICONSET="$ROOT/build/AppIcon.iconset"
TMP="$(mktemp -d)"
trap "rm -rf $TMP" EXIT

echo "Source SVG : $SVG"
echo "Iconset out: $ICONSET"

# 1) Render a 1024x1024 PNG from the SVG (uses QuickLook renderer).
qlmanage -t -s 1024 -o "$TMP" "$SVG" > /dev/null 2>&1
BASE="$TMP/$(basename "$SVG").png"
if [ ! -f "$BASE" ]; then
  echo "Error: qlmanage failed to render $SVG" >&2
  exit 1
fi

# 2) Produce every size required by macOS .iconset
mkdir -p "$ICONSET"
gen() {
  local size="$1"; local name="$2"
  sips -z "$size" "$size" "$BASE" --out "$ICONSET/$name" > /dev/null
}
gen  16  icon_16x16.png
gen  32  icon_16x16@2x.png
gen  32  icon_32x32.png
gen  64  icon_32x32@2x.png
gen  64  icon_64x64.png
gen 128  icon_64x64@2x.png
gen 128  icon_128x128.png
gen 256  icon_128x128@2x.png
gen 256  icon_256x256.png
gen 512  icon_256x256@2x.png
gen 512  icon_512x512.png
gen 1024 icon_512x512@2x.png

# 3) Pack into AppIcon.icns
iconutil -c icns "$ICONSET" -o "$ROOT/build/AppIcon.icns"

# 4) Copy into the existing app bundles (if present)
for BUNDLE in \
  "$ROOT/build/LocalAiAssistant.app/Contents/Resources/AppIcon.icns" \
  "$ROOT/build/dmg-staging/LocalAiAssistant.app/Contents/Resources/AppIcon.icns" \
  "$ROOT/dist/LocalAiAssistant.app/Contents/Resources/AppIcon.icns"
do
  if [ -f "$BUNDLE" ]; then
    cp "$ROOT/build/AppIcon.icns" "$BUNDLE"
    echo "Updated bundle icon: $BUNDLE"
  fi
done

echo "Done. AppIcon.icns generated at $ROOT/build/AppIcon.icns"
