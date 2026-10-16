# ADR-0001: Frame transport for the player

- **Status:** Accepted (Phase 1 spike: **pass**, 2026-10-13)
- **Spike:** a throwaway crate (`spike-display`), removed after this decision; the numbers below are its measurements.

## Context

The player window renders the emulator screen and sends input back. The budget is **≥ 30 fps** at 1080×2400 downscaled to a 400 px-wide window and **≤ 80 ms** median input latency on an M-series Mac. Per D20 we try the simplest path first, inline RGBA frames over gRPC into a Slint `SharedPixelBuffer`, and move to MMAP transport or a wgpu texture import only if the budget is missed. egui is the documented fallback.

## What was measured

Setup: MacBook Pro M1 Pro, emulator 35.6.11, spike-only AVD (API 31 `google_apis` arm64, 1080×2400, 420 dpi, 4 cores, 2 GB), launched `-no-window -no-boot-anim -no-snapshot -no-audio -grpc <port> -grpc-use-token`, Slint 1.18 (default winit + femtovg backend), release build.

Pipeline: `streamScreenshot(RGBA8888, width, height)` → newest-frame slot (one UI hop in flight; newer frames replace older ones) → `slint::invoke_from_event_loop` → `SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice` → `Image`. Input: Slint `TouchArea` → device pixels → client-streaming `streamInputEvent` (touch, pressure 0 on release, `NEVER_EXPIRE`).

- **fps bench** (`--bench fps`): opens Settings, injects alternating 300 ms drags over gRPC for 10 s. "Active fps" = 1 / median frame interval, gaps > 100 ms between drags excluded.
- **latency bench** (`--bench latency`): `show_touches=1`, home screen, 30 taps; time from sending the touch-down to the first frame on the UI thread whose probe pixel changed (the show-taps dot).

| `-gpu` | Frame size | Active fps (median / p10) | Throughput | Frame age (emulator ts → UI) | Touch → changed frame on UI thread (median / p90) | CPU emulator / player |
|---|---|---|---|---|---|---|
| `host` | 400 px (399×888) | **59.9** / 55.9 | 84 MB/s | 2.0 ms | **13.5 / 16.1 ms** | 280% / 17% |
| `host` | 800 px (799×1777, Retina 2×) | **59.9** / 56.5 | 329 MB/s | 5.4 ms | **13.5 / 25.7 ms** | 315% / 40% |
| `auto` | 800 px | 39.8 / 25.3 | 212 MB/s | 15.4 ms | 31.7 / 43.6 ms | 610% / 26% |
| `auto` | 400 px | ~35 | 49 MB/s | 10.7 ms | — | 583% / 11% |
| `auto` | width only (no downscale, 1080×2400) | 26.6 avg | 276 MB/s | 25.9 ms | — | 545% / 33% |

Estimated input-to-photon with `-gpu host`: 13.5 ms (measured) + up to one 60 Hz vsync to present (≤ 16.7 ms) + host input delivery (a few ms) ≈ **35 ms**, well under 80 ms. The probe doesn't include the OS input path or scanout, so 35 ms is an estimate with those as upper bounds.

## Decision

1. **Inline RGBA8888 over gRPC + Slint `SharedPixelBuffer`.** It passes with a wide margin. MMAP transport and wgpu texture import are **not needed**, and egui stays a fallback only.
2. **Launch emulators with `-gpu host`** on macOS. Under `-no-window`, `auto` falls back to software rendering: 2× the emulator CPU, ~40 fps, 2–3× the latency. The backend (Phase 3b) owns this default. A device setting may override it later (e.g. `swiftshader_indirect` for CI or broken GPU drivers).
3. **Always request both `width` and `height`** in `ImageFormat`. The emulator ignores a width-only request and sends full-resolution frames (finding, emulator 35.6.11). It keeps the aspect ratio and may round down by 1 px (400 → 399×888).
4. **Request frames at the window's physical pixel size** (logical width × scale factor, capped at the device size). Re-subscribe on resize or rotation.
5. **Keep only the newest frame**: one `invoke_from_event_loop` in flight, and frames arriving meanwhile overwrite the slot. The UI never builds a backlog.
6. Frames arrive **top-down** (row 0 = top), despite the proto comment saying "bottom up". No flip is needed.

## Consequences

- The player window (Phase 7c) is built on this path and uses `device.connect_info` for the gRPC endpoint + token.
- The backend (Phase 3b) launches with `-gpu host` by default and exposes the GPU mode in `LaunchOptions`.
- Streaming at 2× Retina costs ~330 MB/s on loopback and ~40% of one core in the player. Acceptable. Re-check if multiple players are open at once.
- CI (Linux KVM, no GPU) needs `-gpu swiftshader_indirect`. CI only runs functional tests, not the fps budget.

## Not verified by the spike

- Manual check "tap Settings and confirm it opens" through a mouse click in the window. The spike's scripted gRPC drags and taps exercise the same input path, but the mouse → `TouchArea` → device-pixel mapping needs a human click. It remains a manual check in the player window.
- Multi-touch, keyboard and rotation. They belong in Phase 7c.
