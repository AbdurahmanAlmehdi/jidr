import React from 'react';
import {interpolate, interpolateColors, spring} from 'remotion';
import {C, FONT_MONO, FONT_UI} from '../theme';
import {FPS, STEPS, STEP_STARTS, clamp, fade, standard} from '../timeline';
import {Button, Icon, Kbd, Pill} from './primitives';

// The Root sheet (crates/jidr-ui/src/root/root.slint), from root-progress.dark.png to
// root-success.dark.png / docs/images/root-success.png. Sheet-relative coordinates;
// it sits at x 410, y 44 in the 1440×900 window.

export const ROOT_SHEET = {x: 410, y: 44, w: 620, progressH: 606, successH: 525};

type StepState = 'queued' | 'now' | 'done';

const stepState = (frame: number, i: number): {state: StepState; p: number} => {
  const start = STEP_STARTS[i];
  const end = start + STEPS[i].frames;
  if (frame < start) return {state: 'queued', p: 0};
  if (frame >= end) return {state: 'done', p: 1};
  return {state: 'now', p: (frame - start) / STEPS[i].frames};
};

const fmt = (sec: number) => `0:${String(Math.floor(sec)).padStart(2, '0')}`;

const StepRow: React.FC<{i: number; frame: number}> = ({i, frame}) => {
  const st = STEPS[i];
  const {state, p} = stepState(frame, i);
  const doneAt = STEP_STARTS[i] + st.frames;
  // The check pops in when a step completes.
  const pop = state === 'done' ? spring({frame: frame - doneAt, fps: FPS, config: {damping: 12, stiffness: 220}}) : 0;
  const nowIn = state === 'now' ? fade(frame, STEP_STARTS[i], STEP_STARTS[i] + 5) : 0;
  const hasSub = state === 'now' ? Boolean('nowSub' in st ? st.nowSub : st.sub) : Boolean(st.sub);
  const sub = state === 'now' && 'nowSub' in st ? st.nowSub : st.sub;
  const glyphBg = state === 'done' ? C.positiveContainer : state === 'now' ? C.transitionContainer : C.neutralContainer;
  const glyphFg = state === 'done' ? C.positiveOn : state === 'now' ? C.transitionOn : C.neutralOn;
  const trail = state === 'done' ? 'Done' : state === 'now' ? fmt(p * st.real) : 'Queued';
  return (
    <div
      style={{
        position: 'relative',
        height: hasSub ? 50 : 40,
        background: state === 'now' ? `rgba(23,35,63,${nowIn})` : 'transparent',
        borderTop: i === 0 ? 'none' : `1px solid ${C.borderSubtle}`,
        display: 'flex',
        alignItems: 'center',
        padding: '0 14px',
        gap: 11,
        boxSizing: 'border-box',
      }}
    >
      <div
        style={{
          width: 24,
          height: 24,
          borderRadius: 12,
          background: glyphBg,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flex: 'none',
        }}
      >
        {state === 'now' ? (
          <Icon name="loader-circle" size={14} color={glyphFg} style={{transform: `rotate(${frame * 12}deg)`}} />
        ) : state === 'done' ? (
          <Icon name="check" size={14} color={glyphFg} stroke={2.25} style={{transform: `scale(${0.4 + 0.6 * pop})`}} />
        ) : (
          <Icon name="clock" size={14} color={glyphFg} />
        )}
      </div>
      <div style={{flex: 1}}>
        <div style={{color: C.textPrimary, font: `${state === 'now' ? 600 : 400} 13px/17px ${FONT_UI}`}}>
          {state === 'now' ? st.nowTitle : st.title}
        </div>
        {hasSub ? (
          <div
            style={{
              color: C.textSecondary,
              font: `400 12px/16px ${'mono' in st && st.mono ? FONT_MONO : FONT_UI}`,
              marginTop: 1,
            }}
          >
            {sub}
          </div>
        ) : null}
      </div>
      <span
        style={{
          color: state === 'now' ? C.transitionOn : C.textSecondary,
          font: `${state === 'now' ? 500 : 400} 12px/1 ${state === 'now' ? FONT_MONO : FONT_UI}`,
        }}
      >
        {trail}
      </span>
      {state === 'now' ? (
        <div style={{position: 'absolute', left: 0, bottom: 0, height: 2, width: `${p * 100}%`, background: C.info}} />
      ) : null}
    </div>
  );
};

/** `frame` is the global frame; `rootedAt` is when verification succeeds. */
export const RootSheet: React.FC<{frame: number; rootedAt: number}> = ({frame, rootedAt}) => {
  const r = frame - rootedAt;
  // Progress content leaves first, then the result arrives: no two states on top of each other.
  const out = interpolate(r, [0, 6], [0, 1], {...clamp, easing: standard});
  const ok = interpolate(r, [4, 14], [0, 1], {...clamp, easing: standard});
  const pop = spring({frame: r - 4, fps: FPS, config: {damping: 11, stiffness: 170}});
  const height = interpolate(ok, [0, 1], [ROOT_SHEET.progressH, ROOT_SHEET.successH]);
  const curStep = STEP_STARTS.reduce((acc, s, i) => (frame >= s ? i + 1 : acc), 1);
  const ring = interpolate(r, [6, 30], [0, 1], clamp);

  const plateBg = interpolateColors(ok, [0, 1], [C.rootSoftContainer, C.rootStrong]);
  const plateBd = interpolateColors(ok, [0, 1], [C.rootSoftBorder, C.rootStrong]);
  const iconScale = r < 4 ? 1 - 0.1 * out : 0.86 + 0.14 * pop;

  const progressOut = 1 - out;
  const successIn = (d: number) => interpolate(r, [8 + d, 18 + d], [0, 1], {...clamp, easing: standard});

  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        top: 0,
        width: ROOT_SHEET.w,
        height,
        background: C.elev3,
        border: `1px solid ${C.borderDefault}`,
        borderTop: 'none',
        borderRadius: '0 0 14px 14px',
        overflow: 'hidden',
        fontFamily: FONT_UI,
        boxShadow: '0 24px 60px rgba(0,0,0,0.55)',
      }}
    >
      {/* Header */}
      <div
        style={{
          height: 51,
          borderBottom: `1px solid ${C.borderDefault}`,
          display: 'flex',
          alignItems: 'center',
          padding: '0 20px',
          justifyContent: 'space-between',
        }}
      >
        <span style={{color: C.textPrimary, font: `600 15px/20px ${FONT_UI}`}}>Full root · Pixel-ish 6.1</span>
        <span style={{color: C.textSecondary, font: `400 12.5px/1 ${FONT_MONO}`}}>emulator-5556</span>
      </div>

      {/* Plate */}
      <div style={{position: 'absolute', left: 274, top: 72, width: 72, height: 72}}>
        <div
          style={{
            position: 'absolute',
            inset: -ring * 26,
            borderRadius: 16 + ring * 26,
            border: `2px solid ${C.rootStrong}`,
            opacity: r >= 6 ? (1 - ring) * 0.6 : 0,
          }}
        />
        <div
          style={{
            position: 'absolute',
            inset: 0,
            borderRadius: 16,
            background: plateBg,
            border: `1px solid ${plateBd}`,
            boxSizing: 'border-box',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            transform: `scale(${iconScale})`,
            boxShadow: ok > 0 ? `0 0 ${40 * ok}px rgba(167,139,250,${0.35 * ok})` : undefined,
          }}
        >
          <Icon name="key-round" size={34} color={C.rootSoftOn} style={{position: 'absolute', opacity: progressOut}} />
          <Icon name="badge-check" size={36} color={C.rootStrongOn} style={{position: 'absolute', opacity: ok}} />
        </div>
      </div>

      {/* Title */}
      {[
        {t: 'Setting up root…', o: progressOut, dy: -6 * out},
        {t: 'Rooted', o: ok, dy: 8 * (1 - ok)},
      ].map(({t, o, dy}) => (
        <div
          key={t}
          style={{
            position: 'absolute',
            left: 0,
            right: 0,
            top: 156,
            textAlign: 'center',
            color: C.textPrimary,
            font: `600 32px/38px ${FONT_UI}`,
            letterSpacing: -0.8,
            opacity: o,
            transform: `translateY(${dy}px)`,
          }}
        >
          {t}
        </div>
      ))}

      {/* Badge */}
      <div style={{position: 'absolute', left: 0, right: 0, top: 210, display: 'flex', justifyContent: 'center'}}>
        <div style={{position: 'relative', display: 'flex', justifyContent: 'center'}}>
          <Pill tone="rootSoft" outlined icon="loader-circle" spin={frame * 12} style={{opacity: progressOut, background: 'transparent'}}>
            Setting up root… · step {curStep} of 5
          </Pill>
          <Pill
            tone="rootStrong"
            icon="badge-check"
            style={{position: 'absolute', top: 0, opacity: ok, transform: `scale(${0.9 + 0.1 * pop})`}}
          >
            Rooted · Magisk 30.7 · verified just now
          </Pill>
        </div>
      </div>

      {/* Body line */}
      <div
        style={{
          position: 'absolute',
          left: 80,
          right: 80,
          top: 245,
          textAlign: 'center',
          color: C.textSecondary,
          font: `400 13px/17px ${FONT_UI}`,
          opacity: progressOut,
        }}
      >
        Reboots the device to finish root setup (about 40 s). Leave it running; you can close this sheet.
      </div>
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          top: 245,
          textAlign: 'center',
          color: C.textSecondary,
          font: `400 13px/17px ${FONT_UI}`,
          opacity: successIn(0),
        }}
      >
        Jidr checked that su really answers as root before saying so.
      </div>

      {/* Steps */}
      <div
        style={{
          position: 'absolute',
          left: 21,
          top: 297,
          width: 578,
          borderRadius: 8,
          background: C.surfaceRaised,
          overflow: 'hidden',
          opacity: progressOut,
          transform: `translateY(${-10 * out}px)`,
        }}
      >
        {STEPS.map((_, i) => (
          <StepRow key={i} i={i} frame={frame} />
        ))}
      </div>

      {/* Result table */}
      <div
        style={{
          position: 'absolute',
          left: 20,
          top: 279,
          width: 580,
          height: 114,
          borderRadius: 8,
          background: C.surfaceSunken,
          boxSizing: 'border-box',
          padding: '11px 14px',
          opacity: successIn(1),
        }}
      >
        {[
          ['Check', 'su -c id → uid=0(root) gid=0(root)'],
          ['Magisk', '30.7 · app installed'],
          ['Boot image', 'patched · original kept'],
          ['Took', '58 s · 5 of 5 steps'],
        ].map(([k, v], i) => (
          <div
            key={k}
            style={{
              display: 'flex',
              height: 24,
              alignItems: 'center',
              opacity: successIn(2 + i * 3),
              transform: `translateY(${6 * (1 - successIn(2 + i * 3))}px)`,
            }}
          >
            <span style={{width: 104, color: C.textSecondary, fontSize: 12.5}}>{k}</span>
            <span style={{color: C.textPrimary, font: `400 12.5px/1 ${FONT_MONO}`}}>{v}</span>
          </div>
        ))}
      </div>

      {/* Play Integrity note */}
      <div
        style={{
          position: 'absolute',
          left: 20,
          top: 409,
          width: 580,
          height: 40,
          borderRadius: 8,
          boxSizing: 'border-box',
          background: C.warningContainer,
          border: `1px solid ${C.warningBorder}`,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '0 14px',
          color: C.warningOn,
          fontSize: 12,
          opacity: successIn(12),
        }}
      >
        <Icon name="shield-alert" size={15} color={C.warningOn} />
        Rooted emulators can't pass Play Integrity. Apps that check it will refuse to run.
      </div>

      {/* Footer */}
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          bottom: 0,
          height: 55,
          background: C.surfaceChrome,
          borderTop: `1px solid ${C.borderDefault}`,
        }}
      >
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            padding: '0 24px 0 28px',
            opacity: progressOut,
          }}
        >
          <span style={{flex: 1, color: C.textSecondary, fontSize: 13}}>Stop and revert</span>
          <Button>Continue in background</Button>
        </div>
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '0 16px 0 28px',
            opacity: successIn(6),
          }}
        >
          <span style={{flex: 1, color: C.textSecondary, fontSize: 13}}>Show steps</span>
          <Button>Save snapshot…</Button>
          <div
            style={{
              height: 30,
              padding: '0 8px 0 12px',
              borderRadius: 8,
              background: C.accentFill,
              color: '#fff',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              fontSize: 13,
              fontWeight: 500,
            }}
          >
            Done <Kbd onAccent>↵</Kbd>
          </div>
        </div>
      </div>
    </div>
  );
};
