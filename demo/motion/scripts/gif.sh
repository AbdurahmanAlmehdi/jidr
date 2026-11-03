#!/bin/sh
# README GIF from the rendered video: PNG frames at WIDTH px and FPS, then gifski.
# Usage (from demo/motion): sh scripts/gif.sh [width] [fps] [quality]   (defaults 960 15 80)
# Frames stay in out/gif-frames-<width>-<fps>/ (git-ignored with the rest of out/).
set -eu
WIDTH="${1:-960}"
FPS="${2:-15}"
QUALITY="${3:-80}"
FRAMES="out/gif-frames-$WIDTH-$FPS"
mkdir -p "$FRAMES"
ffmpeg -v error -y -i out/jidr-demo-motion.mp4 -vf "fps=$FPS,scale=$WIDTH:-1:flags=lanczos" "$FRAMES/f%04d.png"
gifski --fps "$FPS" --width "$WIDTH" --quality "$QUALITY" -o out/jidr-demo-motion.gif "$FRAMES"/f*.png
ls -l out/jidr-demo-motion.gif
