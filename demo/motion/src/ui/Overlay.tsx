import React from 'react';
import {interpolate, useCurrentFrame} from 'remotion';
import {C, FONT_UI} from '../theme';
import {clamp, enter, fade} from '../timeline';

/** macOS-style arrow pointer; (x, y) is the tip in stage pixels. */
export const Cursor: React.FC<{x: number; y: number; opacity: number; clicks: number[]}> = ({x, y, opacity, clicks}) => {
  const frame = useCurrentFrame();
  const last = clicks.filter((c) => c <= frame).pop();
  const since = last === undefined ? 99 : frame - last;
  const press = since < 5 ? interpolate(since, [0, 2, 5], [1, 0.82, 1], clamp) : 1;
  const ring = interpolate(since, [0, 14], [0, 1], clamp);
  return (
    <div style={{position: 'absolute', left: x, top: y, opacity, pointerEvents: 'none'}}>
      {since < 14 ? (
        <div
          style={{
            position: 'absolute',
            left: -36 * ring,
            top: -36 * ring,
            width: 72 * ring,
            height: 72 * ring,
            borderRadius: '50%',
            background: `rgba(122,162,255,${0.28 * (1 - ring)})`,
            border: `2px solid rgba(122,162,255,${0.7 * (1 - ring)})`,
            boxSizing: 'border-box',
          }}
        />
      ) : null}
      <svg
        width={30}
        height={40}
        viewBox="0 0 15 20"
        style={{position: 'absolute', left: -2, top: -1, transform: `scale(${press})`, transformOrigin: '2px 1px', filter: 'drop-shadow(0 2px 3px rgba(0,0,0,0.45))'}}
      >
        <path
          d="M1.5 1 L1.5 16.2 L5.3 12.7 L7.9 18.6 L10.4 17.5 L7.9 11.8 L13 11.8 Z"
          fill="#0b0b0c"
          stroke="#ffffff"
          strokeWidth={1.2}
          strokeLinejoin="round"
        />
      </svg>
    </div>
  );
};

/** Burned-in caption: one style, bottom centre. */
export const Caption: React.FC<{text: string; from: number; to: number}> = ({text, from, to}) => {
  const frame = useCurrentFrame();
  if (frame < from - 1 || frame > to + 10) return null;
  const inP = enter(frame, from, 14);
  const outP = fade(frame, to, to + 8);
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        right: 0,
        bottom: 40,
        display: 'flex',
        justifyContent: 'center',
        opacity: inP * (1 - outP),
        transform: `translateY(${14 * (1 - inP) + 6 * outP}px)`,
      }}
    >
      <div
        style={{
          padding: '14px 32px',
          borderRadius: 999,
          background: 'rgba(19,22,27,0.92)',
          border: `1px solid ${C.borderDefault}`,
          boxShadow: '0 12px 40px rgba(0,0,0,0.5)',
          color: C.textPrimary,
          font: `500 36px/44px ${FONT_UI}`,
          letterSpacing: -0.4,
          whiteSpace: 'nowrap',
        }}
      >
        {text}
      </div>
    </div>
  );
};

/** Time-lapse marker for the sped-up root steps (not part of the app UI). */
export const SpeedChip: React.FC<{from: number; to: number; label: string}> = ({from, to, label}) => {
  const frame = useCurrentFrame();
  if (frame < from || frame > to + 10) return null;
  const o = fade(frame, from, from + 8) * (1 - fade(frame, to, to + 8));
  return (
    <div
      style={{
        position: 'absolute',
        left: 56,
        top: 48,
        opacity: o,
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '10px 20px 10px 16px',
        borderRadius: 999,
        background: 'rgba(19,22,27,0.92)',
        border: `1px solid ${C.borderDefault}`,
        color: C.textSecondary,
        font: `500 26px/1 ${FONT_UI}`,
      }}
    >
      <svg width={28} height={20} viewBox="0 0 28 20">
        <path d="M2 2 L13 10 L2 18 Z M14 2 L25 10 L14 18 Z" fill={C.accentText} />
      </svg>
      {label}
    </div>
  );
};
