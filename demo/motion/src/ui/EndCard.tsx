import React from 'react';
import {Img, interpolate, spring, staticFile, useCurrentFrame} from 'remotion';
import {C, FONT_AR, FONT_MONO, FONT_UI} from '../theme';
import {FPS, T, clamp, standard} from '../timeline';

// End card: app icon (assets/brand/app-icon.svg), wordmark, the site's hero line, install command.

const rise = (frame: number, at: number) => {
  const p = interpolate(frame, [at, at + 14], [0, 1], {...clamp, easing: standard});
  return {opacity: p, transform: `translateY(${18 * (1 - p)}px)`};
};

export const EndCard: React.FC = () => {
  const frame = useCurrentFrame();
  const t = T.endIn;
  const icon = spring({frame: frame - (t + 4), fps: FPS, config: {damping: 14, stiffness: 120}});
  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        fontFamily: FONT_UI,
      }}
    >
      <Img
        src={staticFile('app-icon.svg')}
        style={{
          width: 250,
          height: 250,
          opacity: Math.min(1, icon * 1.5),
          transform: `scale(${0.72 + 0.28 * icon})`,
        }}
      />
      <div style={{display: 'flex', alignItems: 'baseline', gap: 26, marginTop: 6, ...rise(frame, t + 12)}}>
        <span style={{color: C.textPrimary, font: `600 104px/1.05 ${FONT_UI}`, letterSpacing: -3}}>Jidr</span>
        <span style={{color: C.lilac, font: `600 76px/1 ${FONT_AR}`}}>جذر</span>
      </div>
      <div style={{color: C.textSecondary, font: `500 40px/1.3 ${FONT_UI}`, marginTop: 18, letterSpacing: -0.4, ...rise(frame, t + 20)}}>
        Android on your Mac. Rooted in a minute.
      </div>
      <div
        style={{
          marginTop: 52,
          padding: '20px 30px',
          borderRadius: 14,
          background: C.surfaceSunken,
          border: `1px solid ${C.borderDefault}`,
          font: `400 32px/1 ${FONT_MONO}`,
          color: C.textCode,
          whiteSpace: 'nowrap',
          ...rise(frame, t + 30),
        }}
      >
        <span style={{color: C.textTertiary}}>$ </span>
        <span style={{color: C.accentText}}>brew</span> install --cask abdurahmanalmehdi/tap/jidr
      </div>
      <div style={{marginTop: 34, display: 'flex', gap: 18, alignItems: 'center', font: `500 34px/1 ${FONT_UI}`, ...rise(frame, t + 38)}}>
        <span style={{color: C.accentText}}>jidr.ly</span>
        <span style={{color: C.textTertiary}}>·</span>
        <span style={{color: C.textSecondary}}>Free and open source</span>
        <span style={{color: C.textTertiary}}>·</span>
        <span style={{color: C.textSecondary}}>Apple Silicon</span>
      </div>
    </div>
  );
};
