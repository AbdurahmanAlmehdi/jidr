import {Easing, interpolate, spring} from 'remotion';

export const FPS = 30;
export const DURATION = 25 * FPS; // 750
export const STAGE_W = 1920;
export const STAGE_H = 1080;

// The app window is laid out at the goldens' size (crates/jidr-ui/tests/golden, 1440×900).
export const WIN_W = 1440;
export const WIN_H = 900;
export const WIN_SCALE = 1.14;

const s = (sec: number) => Math.round(sec * FPS);

// Storyboard beats, in frames.
export const T = {
  // 1. Library → New device → API 35
  winIn: 0,
  cursorToPlus: s(0.35),
  plusClick: s(1.0),
  sheetOpen: s(1.05),
  apiClick: s(2.3),
  // 2. Root mode → Full root → Create
  fullClick: s(3.55),
  createClick: s(4.6),
  // 3. Root steps
  rootSheetIn: s(5.0),
  stepsStart: s(5.45),
  // 5. Terminal
  termIn: s(15.0),
  // 6. End card
  endIn: s(20.0),
};

// Five Full-root steps, labels verbatim from crates/jidr-ui/src/root/root.slint (StepRow).
// `real` is the step's real duration in seconds (58 s in all, as in root-success.png);
// `frames` is its on-screen duration.
export const STEPS = [
  {title: 'Download Magisk 30.7', nowTitle: 'Download Magisk 30.7', sub: '', real: 6, frames: 26},
  {title: 'Patch boot image', nowTitle: 'Patch boot image', sub: 'Original kept for revert', real: 5, frames: 26},
  {title: 'Reboot', nowTitle: 'Rebooting', sub: '', nowSub: 'Waiting for Android to report boot complete', real: 25, frames: 40},
  {title: 'Install Magisk app', nowTitle: 'Install Magisk app', sub: '', real: 15, frames: 32},
  {title: 'Verify', nowTitle: 'Verify', sub: 'su -c id', mono: true, real: 7, frames: 24},
] as const;

export const STEP_STARTS = STEPS.reduce<number[]>((acc, st, i) => {
  acc.push(i === 0 ? T.stepsStart : acc[i - 1] + STEPS[i - 1].frames);
  return acc;
}, []);
export const STEPS_END = STEP_STARTS[4] + STEPS[4].frames;
// 4. Rooted: the sheet turns into the result a beat after Verify finishes.
export const ROOTED = STEPS_END + 4;

export const clamp = {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'} as const;

// jidr-easing-standard
export const standard = Easing.bezier(0.2, 0, 0, 1);
export const moveEase = Easing.bezier(0.45, 0, 0.2, 1);

export type Key<V> = {f: number; v: V};

/** Holds each value until the next key, then eases to it over `dur` frames ending at that key. */
export function track(frame: number, keys: Key<number[]>[], ease = moveEase): number[] {
  if (frame <= keys[0].f) return keys[0].v;
  for (let i = 1; i < keys.length; i++) {
    const a = keys[i - 1];
    const b = keys[i];
    if (frame <= b.f) {
      const t = interpolate(frame, [a.f, b.f], [0, 1], {...clamp, easing: ease});
      return a.v.map((x, j) => x + (b.v[j] - x) * t);
    }
  }
  return keys[keys.length - 1].v;
}

/** 0→1 spring with no bounce, starting at `start`. */
export function enter(frame: number, start: number, dur = 14, damping = 200): number {
  return spring({frame: frame - start, fps: FPS, durationInFrames: dur, config: {damping}});
}

export function fade(frame: number, from: number, to: number): number {
  return interpolate(frame, [from, to], [0, 1], {...clamp, easing: standard});
}
