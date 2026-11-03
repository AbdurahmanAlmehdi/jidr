# Demo video

Two takes on the same 25-second storyboard: New device → API 35 → Full root → the five root steps at 8× → Rooted → `adb shell su -c id` and `frida-ps -U` → end card.

## `recording/`: the real app (README GIF)

1. Record the Jidr window while you create an API 35 device with Full root: `screencapture -v -V 210 -l<window id> raw.mov`. Then record the Rooted sheet the same way as `rooted.mov`.
2. With that device the only one attached and `frida-server` running as root, render the terminal beat: `vhs demo/recording/terminal.tape`.
3. Run `demo/recording/edit.sh <dir with raw.mov and rooted.mov>`. It writes `demo/recording/out/jidr-demo.mp4` and `docs/images/demo.gif` (gifski, kept under 10 MB).

The cut points in `edit.sh` belong to one particular take. Retime them if you record again.

Needs `ffmpeg`, `gifski`, `vhs`, `imagemagick` and `librsvg` (`brew install ffmpeg gifski vhs imagemagick librsvg`), plus `frida-tools` (`pipx install frida-tools`).

## `motion/`: motion design version (Remotion)

```bash
cd demo/motion && npm ci
npm run studio    # preview
npm run render    # out/jidr-demo-motion.mp4 (1920×1080, 30 fps)
npm run gif       # out/jidr-demo-motion.gif
```

The UI is rebuilt from `docs/design/` and the golden screenshots in `crates/jidr-ui/tests/golden/`.
