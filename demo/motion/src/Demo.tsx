import React from 'react';
import {AbsoluteFill, interpolate, spring, useCurrentFrame} from 'remotion';
import './theme';
import {C} from './theme';
import {
  FPS,
  ROOTED,
  STAGE_H,
  STAGE_W,
  STEPS_END,
  STEP_STARTS,
  T,
  WIN_H,
  WIN_SCALE,
  WIN_W,
  clamp,
  fade,
  standard,
  track,
  type Key,
} from './timeline';
import {AppWindow} from './ui/AppWindow';
import {EndCard} from './ui/EndCard';
import {Caption, Cursor, SpeedChip} from './ui/Overlay';
import {Terminal} from './ui/Terminal';

// Camera: the window point [cx, cy] sits at the stage centre, zoomed by z on top of WIN_SCALE.
const CAMERA: Key<number[]>[] = [
  {f: 0, v: [720, 450, 1]},
  {f: 12, v: [720, 450, 1]},
  {f: 30, v: [650, 400, 1.06]}, // toward Devices +
  {f: 34, v: [650, 400, 1.06]},
  {f: 56, v: [720, 300, 1.5]}, // sheet: name and images
  {f: 78, v: [720, 300, 1.5]},
  {f: 98, v: [720, 470, 1.6]}, // root mode cards
  {f: 116, v: [720, 470, 1.6]},
  {f: 132, v: [760, 545, 1.5]}, // Create
  {f: T.createClick + 4, v: [760, 545, 1.5]},
  {f: T.rootSheetIn + 8, v: [720, 430, 1.08]}, // the device tab opens
  {f: T.rootSheetIn + 14, v: [720, 430, 1.08]},
  {f: STEP_STARTS[0] + 22, v: [720, 410, 1.45]}, // steps
  {f: STEPS_END, v: [720, 415, 1.5]},
  {f: ROOTED + 24, v: [720, 322, 1.68]}, // Rooted
  {f: T.termIn, v: [720, 318, 1.76]},
  {f: T.termIn + 24, v: [720, 450, 0.98]}, // step back for the terminal
];

// Cursor tip in window coordinates.
const CURSOR: Key<number[]>[] = [
  {f: 0, v: [1180, 760]},
  {f: 8, v: [1180, 760]},
  {f: T.plusClick - 3, v: [227, 111]},
  {f: T.plusClick + 16, v: [227, 111]},
  {f: T.apiClick - 4, v: [610, 256]},
  {f: T.apiClick + 8, v: [610, 256]},
  {f: T.fullClick - 5, v: [905, 466]},
  {f: T.fullClick + 6, v: [905, 466]},
  {f: T.createClick - 4, v: [978, 678]},
];
const CLICKS = [T.plusClick, T.apiClick, T.fullClick, T.createClick];

const Backdrop: React.FC = () => (
  <AbsoluteFill
    style={{
      background: `radial-gradient(1200px 700px at 50% 0%, rgba(42,136,250,0.16), rgba(14,16,20,0) 70%),
        radial-gradient(900px 600px at 85% 110%, rgba(167,139,250,0.10), rgba(14,16,20,0) 70%), ${C.surfaceBase}`,
    }}
  />
);

export const Demo: React.FC = () => {
  const frame = useCurrentFrame();

  const [cx, cy, z] = track(frame, CAMERA);
  const k = WIN_SCALE * z;
  const toStage = (x: number, y: number) => [STAGE_W / 2 + (x - cx) * k, STAGE_H / 2 + (y - cy) * k];

  // Window entrance.
  const winIn = spring({frame, fps: FPS, durationInFrames: 18, config: {damping: 200}});

  // Terminal takes over; the app steps back.
  const term = spring({frame: frame - T.termIn, fps: FPS, durationInFrames: 20, config: {damping: 200}});
  const end = interpolate(frame, [T.endIn - 6, T.endIn + 6], [0, 1], {...clamp, easing: standard});

  const [curX, curY] = track(frame, CURSOR);
  const [sx, sy] = toStage(curX, curY);
  const cursorOpacity = fade(frame, 6, 12) * (1 - fade(frame, T.createClick + 6, T.createClick + 12));

  return (
    <AbsoluteFill style={{background: C.surfaceBase, overflow: 'hidden'}}>
      <Backdrop />

      {/* App window under the camera */}
      {end < 1 ? (
        <AbsoluteFill
          style={{
            opacity: 1 - end,
            transform: `translateX(${-150 * term}px) scale(${(1 - 0.1 * term) * (0.96 + 0.04 * winIn)})`,
            filter: term > 0 ? `brightness(${1 - 0.5 * term})` : undefined,
          }}
        >
          <div
            style={{
              position: 'absolute',
              left: 0,
              top: 0,
              width: WIN_W,
              height: WIN_H,
              transformOrigin: '0 0',
              transform: `translate(${STAGE_W / 2}px, ${STAGE_H / 2}px) scale(${k}) translate(${-cx}px, ${-cy}px)`,
            }}
          >
            <AppWindow />
          </div>
        </AbsoluteFill>
      ) : null}

      {cursorOpacity > 0 ? <Cursor x={sx} y={sy} opacity={cursorOpacity} clicks={CLICKS} /> : null}

      {/* Terminal */}
      {frame >= T.termIn && end < 1 ? (
        <AbsoluteFill
          style={{
            alignItems: 'center',
            justifyContent: 'flex-start',
            paddingTop: 150,
            opacity: Math.min(1, term * 2.5) * (1 - end),
            transform: `translateX(${160 * (1 - term)}px) scale(${0.96 + 0.04 * term - 0.03 * end})`,
          }}
        >
          <Terminal />
        </AbsoluteFill>
      ) : null}

      {/* End card */}
      {frame >= T.endIn - 6 ? (
        <AbsoluteFill style={{opacity: end}}>
          <Backdrop />
          <EndCard />
        </AbsoluteFill>
      ) : null}

      <SpeedChip from={STEP_STARTS[0]} to={STEPS_END} label="58 s, sped up" />
      <Caption text="New device" from={8} to={T.fullClick - 18} />
      <Caption text="Full root" from={T.fullClick - 10} to={T.rootSheetIn - 2} />
      <Caption text="Magisk 30.7, patched and verified" from={T.rootSheetIn + 8} to={ROOTED + 4} />
      <Caption text="Works with your tools" from={T.termIn + 16} to={T.endIn - 8} />
    </AbsoluteFill>
  );
};
