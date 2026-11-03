#!/usr/bin/env bash
# Cuts the README demo (25 s, captions burned in) from real recordings:
#   raw.mov   window recording: New device → API 35 → Full root → root steps
#   rooted.mov window recording of the Rooted sheet
#   out/terminal.mp4 from terminal.tape (vhs)
# Recordings come from `screencapture -v -l<window id>` on a 2× display.
# Usage: demo/recording/edit.sh <dir with raw.mov and rooted.mov>
# Output: demo/recording/out/jidr-demo.mp4 and docs/images/demo.gif (< 10 MB).
set -euo pipefail

RAW_DIR=${1:?usage: edit.sh <raw dir>}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=$HERE/out
WORK=$OUT/work
mkdir -p "$WORK"

W=1440 H=900 FPS=30
# The window sits at this offset inside the capture (the rest is its shadow).
CROP="crop=2880:1800:112:76,scale=$W:$H:flags=lanczos"
FONT=/System/Library/Fonts/SFNS.ttf
MONO=/System/Library/Fonts/SFNSMono.ttf

# pill <text> <fg> <bg> <gravity> <geometry> <file>: a rounded label on a
# transparent full-frame overlay.
pill() {
  magick -background none -fill "$2" -font "$FONT" -pointsize 40 label:"$1" "$WORK/label.png"
  local w h
  read -r w h < <(magick identify -format "%w %h\n" "$WORK/label.png")
  magick -size $((w + 64))x$((h + 32)) xc:none -fill "$3" -stroke '#2A2F38' -strokewidth 2 \
    -draw "roundrectangle 1,1 $((w + 62)),$((h + 30)) $(((h + 32) / 2)),$(((h + 32) / 2))" \
    "$WORK/label.png" -gravity center -composite "$WORK/pill.png"
  magick -size ${W}x${H} xc:none "$WORK/pill.png" -gravity "$4" -geometry "$5" -composite "PNG32:$6"
}
caption() { pill "$1" '#E7E9EE' '#0E1014E6' south +0+56 "$2"; }
caption "New device" "$WORK/cap1.png"
caption "Full root" "$WORK/cap2.png"
caption "Magisk 30.7, patched and verified" "$WORK/cap3.png"
# "8×" while the root steps run fast.
pill "8×" '#0E1014' '#7AA2FF' southeast +48+56 "$WORK/chip.png"

# End card.
rsvg-convert -w 168 -h 168 "$ROOT/assets/brand/app-icon.svg" -o "$WORK/icon.png"
magick -size ${W}x${H} xc:'#0E1014' \
  "$WORK/icon.png" -gravity center -geometry +0-170 -composite \
  -font "$FONT" -fill '#E7E9EE' -pointsize 84 -annotate +0-10 'Jidr' \
  -fill '#9BA3B0' -pointsize 34 -annotate +0+62 'Android on your Mac, rooted in a minute.' \
  -fill '#191C22' -draw "roundrectangle 210,560 1230,640 14,14" \
  -font "$MONO" -fill '#E7E9EE' -pointsize 30 -annotate +0+150 'brew install --cask abdurahmanalmehdi/tap/jidr' \
  -font "$FONT" -fill '#7AA2FF' -pointsize 34 -annotate +0+250 'jidr.ly' \
  "$WORK/endcard.png"

# clip <input> <start> <dur> <speed> <out>: cut, crop, retime to $FPS.
clip() {
  ffmpeg -v error -y -ss "$2" -t "$3" -i "$1" \
    -vf "$CROP,setpts=PTS/$4,fps=$FPS,format=yuv420p" -an -c:v libx264 -crf 14 "$5"
}
RAW=$RAW_DIR/raw.mov
clip "$RAW" 2.8 2.8 1.4 "$WORK/s1.mp4"   # sheet opens, list scrolls to API 35
clip "$RAW" 9.6 1.2 1.2 "$WORK/s2.mp4"   # pick API 35 · Google APIs
clip "$RAW" 11.0 1.8 1.5 "$WORK/s3.mp4"  # pick Full root
clip "$RAW" 16.8 0.8 1.0 "$WORK/s4.mp4"  # Create
clip "$RAW" 30.0 48 8 "$WORK/s5.mp4"     # five root steps at 8×
# Rooted: push in on the sheet (centre 1440,612 in the 2880×1800 window) so
# su -c id → uid=0(root) reads at README width. Eased over 0.6 s.
E='(1-pow(1-min(on/18,1),3))'
ffmpeg -v error -y -ss 0.5 -t 4 -i "$RAW_DIR/rooted.mov" \
  -vf "crop=2880:1800:112:76,fps=$FPS,zoompan=z='1+0.565*$E':x='1440-iw/zoom/2':y='900-288*$E-ih/zoom/2':d=1:s=${W}x$H:fps=$FPS,format=yuv420p" \
  -an -c:v libx264 -crf 14 "$WORK/s6.mp4"
ffmpeg -v error -y -i "$OUT/terminal.mp4" \
  -vf "scale=$W:$H,setpts=PTS/1.288,fps=$FPS,format=yuv420p" -t 5 -an -c:v libx264 -crf 14 "$WORK/s7.mp4"
ffmpeg -v error -y -loop 1 -t 5 -i "$WORK/endcard.png" \
  -vf "fps=$FPS,fade=in:st=0:d=0.4,format=yuv420p" -c:v libx264 -crf 14 "$WORK/s8.mp4"

printf "file 's%d.mp4'\n" 1 2 3 4 5 6 7 8 > "$WORK/list.txt"
ffmpeg -v error -y -f concat -safe 0 -i "$WORK/list.txt" -c copy "$WORK/joined.mp4"

ffmpeg -v error -y -i "$WORK/joined.mp4" \
  -i "$WORK/cap1.png" -i "$WORK/cap2.png" -i "$WORK/cap3.png" -i "$WORK/chip.png" \
  -filter_complex "[0][1]overlay=enable='between(t,0,3)'[a];[a][2]overlay=enable='between(t,3,5)'[b];[b][3]overlay=enable='between(t,5,11)'[c];[c][4]overlay=enable='between(t,5,11)',format=yuv420p" \
  -c:v libx264 -crf 18 -movflags +faststart "$OUT/jidr-demo.mp4"

# GIF: gifski from PNG frames, shrinking until it fits under 10 MB.
GIF=$ROOT/docs/images/demo.gif
for spec in "1200 15 85" "960 15 80" "800 12 70"; do
  set -- $spec
  rm -rf "$WORK/frames" && mkdir "$WORK/frames"
  ffmpeg -v error -i "$OUT/jidr-demo.mp4" -vf "fps=$2,scale=$1:-1:flags=lanczos" "$WORK/frames/f%04d.png"
  gifski --quiet --fps "$2" --width "$1" --quality "$3" -o "$GIF" "$WORK"/frames/f*.png
  size=$(stat -f %z "$GIF")
  echo "gif: ${1}px ${2}fps q$3 → $((size / 1024)) KB"
  [ "$size" -lt 10000000 ] && break
done
