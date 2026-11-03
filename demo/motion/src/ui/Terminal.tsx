import React from 'react';
import {useCurrentFrame} from 'remotion';
import {C, FONT_MONO, FONT_UI} from '../theme';
import {T} from '../timeline';
import {TrafficLights} from './primitives';

// A terminal next to the app: root through adb, then Frida over USB.

const CMD1 = 'adb shell su -c id';
const CMD2 = 'frida-ps -U';
const FRAMES_PER_CHAR = 1.25;

// Rows from a real `frida-ps -U` on a Full-root API 35 device (a subset, in
// its sort order).
const PROCS: Array<[number, string]> = [
  [1948, 'Magisk'],
  [1849, 'adbd'],
  [946, 'com.android.phone'],
  [819, 'com.android.systemui'],
  [1, 'init'],
  [352, 'magiskd'],
  [425, 'surfaceflinger'],
  [575, 'system_server'],
  [377, 'zygote64'],
];

const t0 = T.termIn;
const CMD1_AT = t0 + 14;
const CMD1_DONE = CMD1_AT + Math.ceil(CMD1.length * FRAMES_PER_CHAR);
const OUT1_AT = CMD1_DONE + 6;
const CMD2_AT = OUT1_AT + 14;
const CMD2_DONE = CMD2_AT + Math.ceil(CMD2.length * FRAMES_PER_CHAR);
const OUT2_AT = CMD2_DONE + 6;
const ROW_GAP = 2;
const PROMPT3_AT = OUT2_AT + 2 * ROW_GAP + PROCS.length * ROW_GAP + 4;

const typed = (frame: number, text: string, at: number) =>
  text.slice(0, Math.max(0, Math.min(text.length, Math.floor((frame - at) / FRAMES_PER_CHAR))));

const LINE = 36;

const Prompt: React.FC = () => <span style={{color: C.textTertiary}}>$ </span>;

const Block: React.FC<{on: boolean}> = ({on}) => (
  <span
    style={{
      display: 'inline-block',
      width: 14,
      height: 28,
      marginLeft: 2,
      verticalAlign: -5,
      background: C.textPrimary,
      opacity: on ? 0.85 : 0,
    }}
  />
);

export const Terminal: React.FC = () => {
  const frame = useCurrentFrame();
  const blink = Math.floor(frame / 16) % 2 === 0;
  const lines: React.ReactNode[] = [];

  const cmd1 = typed(frame, CMD1, CMD1_AT);
  lines.push(
    <div key="c1">
      <Prompt />
      <span style={{color: C.textPrimary}}>{cmd1}</span>
      {frame < OUT1_AT ? <Block on={frame < CMD1_DONE || blink} /> : null}
    </div>,
  );
  if (frame >= OUT1_AT) {
    lines.push(
      <div key="o1" style={{color: C.textCode}}>
        <span style={{color: C.positive}}>uid=0(root) gid=0(root)</span> groups=0(root) context=u:r:magisk:s0
      </div>,
    );
  }
  if (frame >= CMD2_AT - 8) {
    lines.push(
      <div key="c2" style={{marginTop: 14}}>
        <Prompt />
        <span style={{color: C.textPrimary}}>{typed(frame, CMD2, CMD2_AT)}</span>
        {frame < OUT2_AT ? <Block on={frame < CMD2_DONE || frame < CMD2_AT || blink} /> : null}
      </div>,
    );
  }
  if (frame >= OUT2_AT) {
    lines.push(
      <div key="h" style={{color: C.textSecondary, whiteSpace: 'pre'}}>
        {'  PID  Name'}
      </div>,
    );
  }
  if (frame >= OUT2_AT + ROW_GAP) {
    lines.push(
      <div key="hr" style={{color: C.textTertiary, whiteSpace: 'pre'}}>
        {'-----  ------------------------'}
      </div>,
    );
  }
  PROCS.forEach(([pid, name], i) => {
    if (frame >= OUT2_AT + (i + 2) * ROW_GAP) {
      const hot = name === 'magiskd';
      lines.push(
        <div key={name} style={{color: hot ? C.root : C.textCode, whiteSpace: 'pre'}}>
          {String(pid).padStart(5, ' ')}
          {'  '}
          {name}
        </div>,
      );
    }
  });
  if (frame >= PROMPT3_AT) {
    lines.push(
      <div key="c3">
        <Prompt />
        <Block on={blink} />
      </div>,
    );
  }

  return (
    <div
      style={{
        width: 1180,
        height: 712,
        borderRadius: 14,
        background: C.surfaceSunken,
        border: `1px solid ${C.borderDefault}`,
        boxShadow: '0 0 0 1px rgba(255,255,255,0.06), 0 40px 120px rgba(0,0,0,0.65)',
        overflow: 'hidden',
        position: 'relative',
      }}
    >
      <div
        style={{
          height: 52,
          background: C.surfaceChrome,
          borderBottom: `1px solid ${C.borderDefault}`,
          position: 'relative',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: C.textSecondary,
          font: `500 20px/1 ${FONT_UI}`,
        }}
      >
        <TrafficLights x={22} y={19} />
        Terminal · zsh
      </div>
      <div style={{padding: '24px 32px', font: `400 25px/${LINE}px ${FONT_MONO}`}}>{lines}</div>
    </div>
  );
};
