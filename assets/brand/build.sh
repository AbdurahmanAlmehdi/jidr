#!/usr/bin/env bash
# Regenerates every Jidr brand/icon asset from source/appLogoOnPaper.jpg.
# Requires: potrace, rsvg-convert (librsvg), magick (ImageMagick), iconutil (macOS).
#   brew install potrace librsvg imagemagick
# Usage: assets/brand/build.sh   (run from anywhere; outputs are committed)
set -euo pipefail

BRAND="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$BRAND/../.." && pwd)"
GUI="$ROOT/apps/jidr-gui/assets"
WEB="$BRAND/web"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Brand colours (sampled from the approved board, source/brand-board.jpg).
TOP="#2A88FA"; BOTTOM="#1267E0"; LILAC="#B9A6F7"; INK="#101010"

mkdir -p "$WEB" "$GUI/macos" "$GUI/menubar" "$GUI/icons"

# 1. Trace the symbol (regular and a bolder cut for ≤32 px).
trace() { # $1 = extra morphology, $2 = output svg
  magick "$BRAND/source/appLogoOnPaper.jpg" -colorspace Gray -blur 0x1 -threshold 45% \
    -morphology Close Disk:1 $1 -trim +repage -bordercolor white -border 4 "$TMP/s.pbm"
  potrace "$TMP/s.pbm" -s --flat -a 1.0 -O 0.4 -t 8 -o "$TMP/raw.svg"
  local vb; vb=$(grep -o 'viewBox="[^"]*"' "$TMP/raw.svg")
  { echo "<svg xmlns=\"http://www.w3.org/2000/svg\" $vb>"
    sed -n '/<g /,/<\/g>/p' "$TMP/raw.svg" | sed 's/fill="#000000"/fill="currentColor"/'
    echo "</svg>"; } > "$2"
}
trace "" "$BRAND/symbol.svg"
trace "-morphology Erode Disk:14" "$BRAND/symbol-bold.svg"   # Erode on black-on-white = thicker strokes

inner() { sed -n '/<g /,/<\/g>/p' "$1"; }
vbox()  { grep -o 'viewBox="[^"]*"' "$1" | head -1; }

# 2. macOS app icon master: 1024 canvas, 824 squircle body, symbol ≈60% of body.
app_icon() { # $1 = symbol svg, $2 = output svg, $3 = symbol width, $4 = body inset, $5 = shadow (1/0)
  local sw=$3 inset=${4:-100} shadow=${5:-1} sh x y body rx filt=""
  sh=$(( sw * 879 / 851 )); x=$(( 512 - sw / 2 )); y=$(( 512 - sh / 2 ))
  body=$(( 1024 - 2 * inset )); rx=$(( body * 185 / 824 ))
  [ "$shadow" = 1 ] && filt=' filter="url(#shadow)"'
  cat > "$2" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="$TOP"/><stop offset="1" stop-color="$BOTTOM"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity="0.28"/>
    </filter>
  </defs>
  <rect x="$inset" y="$inset" width="$body" height="$body" rx="$rx" fill="url(#bg)"$filt/>
  <svg x="$x" y="$y" width="$sw" height="$sh" $(vbox "$1") color="#FFFFFF">$(inner "$1")</svg>
</svg>
EOF
}
app_icon "$BRAND/symbol.svg"      "$BRAND/app-icon.svg"       494 100 1
app_icon "$BRAND/symbol-bold.svg" "$BRAND/app-icon-small.svg" 640 40  0   # ≤32 px: no shadow, fuller body

# 3. Circular avatar (GitHub org, social).
cat > "$BRAND/avatar.svg" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="$TOP"/><stop offset="1" stop-color="$BOTTOM"/></linearGradient></defs>
  <circle cx="512" cy="512" r="512" fill="url(#bg)"/>
  <svg x="282" y="274" width="460" height="475" $(vbox "$BRAND/symbol.svg") color="#FFFFFF">$(inner "$BRAND/symbol.svg")</svg>
</svg>
EOF

# 4. Social preview card 1200×630.
cat > "$BRAND/social-card.svg" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630">
  <defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="$TOP"/><stop offset="1" stop-color="$BOTTOM"/></linearGradient></defs>
  <rect width="1200" height="630" fill="$INK"/>
  <circle cx="250" cy="315" r="160" fill="url(#bg)"/>
  <svg x="178" y="241" width="144" height="149" $(vbox "$BRAND/symbol.svg") color="#FFFFFF">$(inner "$BRAND/symbol.svg")</svg>
  <text x="470" y="285" font-family="Helvetica Neue, Helvetica, Arial, sans-serif" font-weight="700" font-size="120" fill="#FFFFFF">Jidr</text>
  <text x="474" y="390" font-family="Geeza Pro, SF Arabic, Noto Naskh Arabic, sans-serif" font-weight="700" font-size="92" fill="$LILAC">جذر</text>
  <text x="472" y="470" font-family="Helvetica Neue, Helvetica, Arial, sans-serif" font-size="32" fill="#C9C9C9">Android on your Mac. Rooted in a minute.</text>
</svg>
EOF

# 5. Raster masters.
rsvg-convert -w 1024 -h 1024 "$BRAND/app-icon.svg" -o "$BRAND/app-icon-1024.png"
rsvg-convert -w 512 -h 512   "$BRAND/app-icon.svg" -o "$BRAND/app-icon-512.png"  # the MCP bundle icon
rsvg-convert -w 1024 -h 1024 "$BRAND/avatar.svg"   -o "$BRAND/avatar-1024.png"
rsvg-convert -w 1200 -h 630  "$BRAND/social-card.svg" -o "$BRAND/social-card.png"

# 6. macOS .icns (bold cut for 16/32 px).
SET="$TMP/AppIcon.iconset"; mkdir -p "$SET"
for s in 16 32 128 256 512; do
  src="$BRAND/app-icon.svg"; [ "$s" -le 32 ] && src="$BRAND/app-icon-small.svg"
  rsvg-convert -w $s -h $s "$src" -o "$SET/icon_${s}x${s}.png"
  d=$((s*2)); src2="$BRAND/app-icon.svg"; [ "$d" -le 32 ] && src2="$BRAND/app-icon-small.svg"
  rsvg-convert -w $d -h $d "$src2" -o "$SET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$SET" -o "$GUI/macos/Jidr.icns"
cp "$BRAND/app-icon-1024.png" "$GUI/macos/AppIcon-1024.png"

# 7. Menu bar template (black + alpha; macOS tints it). 18 pt @1x/@2x.
mk_template() { # $1 = px
  local pad=$(( $1 / 9 ))
  rsvg-convert -h $(( $1 - 2*pad )) "$BRAND/symbol-bold.svg" -o "$TMP/t.png"
  magick "$TMP/t.png" -background none -gravity center -extent "${1}x${1}" "$2"
}
mk_template 18 "$GUI/menubar/MenuBarTemplate.png"
mk_template 36 "$GUI/menubar/MenuBarTemplate@2x.png"

# 8. In-app assets for Slint (SVG renders natively; use `colorize` to tint the symbol).
cp "$BRAND/symbol.svg" "$GUI/icons/symbol.svg"
rsvg-convert -w 256 -h 256 "$BRAND/app-icon.svg" -o "$GUI/icons/app-icon-256.png"

# 9. Web set (copied into site/public).
sed "s/currentColor/$BOTTOM/" "$BRAND/symbol-bold.svg" > "$WEB/favicon.svg"
rsvg-convert -w 32 -h 32 "$BRAND/app-icon-small.svg" -o "$WEB/favicon-32.png"
rsvg-convert -w 16 -h 16 "$BRAND/app-icon-small.svg" -o "$TMP/f16.png"
magick "$TMP/f16.png" "$WEB/favicon-32.png" "$WEB/favicon.ico"
rsvg-convert -w 180 -h 180 "$BRAND/app-icon.svg" -o "$WEB/apple-touch-icon.png"
rsvg-convert -w 512 -h 512 "$BRAND/avatar.svg" -o "$WEB/avatar-512.png"
cp "$BRAND/social-card.png" "$WEB/og-image.png"
cp "$BRAND/symbol.svg" "$WEB/symbol.svg"

echo "Brand assets rebuilt."
