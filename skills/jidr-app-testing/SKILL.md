---
name: jidr-app-testing
description: "Test an Android app on a Jidr emulator: install an APK or run a Flutter or React Native app on the device's adb serial, then check a flow with screenshot, tap, swipe, type and the UI tree, and clean up the devices you created. Use when the user asks to run their Android app on an emulator and check that a screen or flow works, reproduce a bug on a device, or verify a UI change. Needs the jidr skill for creating and starting devices."
---

# Testing an app on a Jidr device

Use the jidr skill to get a running device first. Note its adb serial (`emulator-55xx`): `jidr device list` shows it in the SERIAL column. Use that serial everywhere below, never a guessed `emulator-5554`, which may be a device the user is using.

## 1. Get the app onto the device

An APK:

```bash
jidr install Phone build/app-release.apk --grant     # --grant: all runtime permissions
jidr call app.launch '{"device":"Phone","package":"com.example.app"}'
```

Flutter (debug build with hot reload; the serial is the `-d` id):

```bash
flutter run -d emulator-5556
```

React Native: start Metro, then run `npx react-native run-android` with the serial (`--deviceId emulator-5556` on older CLIs, `--device emulator-5556` on newer ones; `--help` says which). With Expo, `npx expo run:android --device` asks which device to use.

For repeatable runs, turn animations off first:

```bash
jidr call dev.set '{"device":"Phone","animation_scale":0}'
```

## 2. Check the flow: look, act, look again

Repeat until the flow is done: take a screenshot, find the element, act on it, take another screenshot to see what happened. Don't act twice without looking in between.

With the Jidr MCP tools (when connected):

| Step | Tool |
|---|---|
| Look | `screenshot {device}`: the image, fitted in 1024×1024 |
| Find | `ui_dump {device}`: uiautomator XML with `text`, `resource-id` and `bounds` |
| Tap / swipe | `tap {device, x, y}`, `swipe {device, x1, y1, x2, y2}`: coordinates in the screenshot image (or `coords: "device"` for device pixels) |
| Type | `type_text {device, text, enter?}` into the focused field |
| Keys | `press_key {device, key}`: `GoBack`, `GoHome`, `Enter`… |
| Logs | `logcat_tail {device, lines?, filter?}` |

With the CLI and adb (device pixels; the screenshot is full size, so its coordinates match):

```bash
jidr --json call media.screenshot '{"device":"Phone"}'    # read result.path to see it
adb -s emulator-5556 exec-out uiautomator dump /dev/tty     # UI tree
adb -s emulator-5556 shell input tap 540 1200
adb -s emulator-5556 shell input swipe 540 1600 540 600 300
adb -s emulator-5556 shell input text 'hello%sworld'        # %s is a space
jidr call key.press '{"device":"Phone","key":"back"}'
jidr logcat Phone -n 200 --filter flutter:V
```

Tips:

- In the UI tree, `bounds="[x1,y1][x2,y2]"`: tap the centre. Prefer finding elements by `text` or `resource-id` over guessing from the picture.
- `ui_dump` takes several seconds. Use screenshots to follow progress and the tree only to find what to tap.
- If the app crashed or the screen didn't change, read the log before trying again: `jidr logcat Phone -n 200` (look for `FATAL EXCEPTION` or `AndroidRuntime`).
- A rotation check: `jidr call sensor.rotate '{"device":"Phone","angle":90}'`, screenshot, then back to `0`.
- To retry a flow from the same state, save a snapshot before it and load it between tries (`jidr snapshot save Phone start`, `jidr snapshot load Phone start`).

Report what you checked with the final screenshot, and the log lines for any failure.

## 3. Clean up

Delete only the devices you created in this session (D5: devices you create are yours to delete). Leave the user's own devices running and untouched, and say what you removed.

```bash
jidr device stop Phone
jidr device delete Phone --yes
```

If the user wants to keep looking at the result, leave your device running and tell them its name instead.
