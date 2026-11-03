import React from 'react';
import {useCurrentFrame} from 'remotion';
import {C, FONT_MONO, FONT_UI} from '../theme';
import {ROOTED, STEP_STARTS, T, WIN_H, WIN_W, enter, fade} from '../timeline';
import {CreateSheet, SHEET} from './CreateSheet';
import {Detail, Sidebar} from './Library';
import {Player} from './Player';
import {Icon, Pill, TrafficLights} from './primitives';
import {ROOT_SHEET, RootSheet} from './RootSheet';

const TITLEBAR = 44;

const Tab: React.FC<{active: number; children: React.ReactNode; width: number; opacity?: number}> = ({
  active,
  children,
  width,
  opacity = 1,
}) => (
  <div
    style={{
      position: 'relative',
      height: 33,
      width,
      marginTop: 11,
      borderRadius: '8px 8px 0 0',
      boxSizing: 'border-box',
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      padding: '0 12px',
      color: active > 0.5 ? C.textPrimary : C.textSecondary,
      fontSize: 13,
      opacity,
      overflow: 'hidden',
      whiteSpace: 'nowrap',
      flex: 'none',
    }}
  >
    <div
      style={{
        position: 'absolute',
        inset: 0,
        bottom: -1,
        borderRadius: '8px 8px 0 0',
        background: C.surfaceBase,
        border: `1px solid ${C.borderDefault}`,
        borderBottom: 'none',
        opacity: active,
      }}
    />
    <div style={{position: 'relative', display: 'flex', alignItems: 'center', gap: 8}}>{children}</div>
  </div>
);

export const AppWindow: React.FC = () => {
  const frame = useCurrentFrame();
  const rootedAt = ROOTED;

  // Library → New device sheet
  const plusHover = fade(frame, T.plusClick - 6, T.plusClick - 2);
  const plusPressed = frame >= T.plusClick && frame < T.plusClick + 4;
  const sheetIn = enter(frame, T.sheetOpen, 12);
  const sheetOut = fade(frame, T.createClick + 2, T.createClick + 8);
  const sheetVisible = sheetIn * (1 - sheetOut);
  const api35 = fade(frame, T.apiClick, T.apiClick + 4);
  const rowHover = fade(frame, T.apiClick - 8, T.apiClick - 3);
  const full = fade(frame, T.fullClick, T.fullClick + 5);
  const fullHover = fade(frame, T.fullClick - 8, T.fullClick - 3);
  const createHover = fade(frame, T.createClick - 6, T.createClick - 2);
  const createPressed = frame >= T.createClick && frame < T.createClick + 4;

  // → device tab + Root sheet
  const newTab = enter(frame, T.createClick + 4, 12);
  const toPlayer = fade(frame, T.createClick + 8, T.createClick + 12);
  const rootIn = enter(frame, T.rootSheetIn, 14);
  const rooted = frame >= rootedAt;
  const screenDark =
    fade(frame, STEP_STARTS[2], STEP_STARTS[2] + 4) * (1 - fade(frame, STEP_STARTS[3] + 2, STEP_STARTS[3] + 8));

  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        top: 0,
        width: WIN_W,
        height: WIN_H,
        borderRadius: 12,
        overflow: 'hidden',
        background: C.surfaceBase,
        boxShadow: '0 0 0 1px rgba(255,255,255,0.08), 0 40px 120px rgba(0,0,0,0.6)',
        fontFamily: FONT_UI,
      }}
    >
      {/* Title bar and tabs */}
      <div
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          right: 0,
          height: TITLEBAR,
          background: C.surfaceChrome,
          borderBottom: `1px solid ${C.borderDefault}`,
          boxSizing: 'border-box',
        }}
      >
        <TrafficLights />
        <div style={{position: 'absolute', left: 86, top: 0, display: 'flex', gap: 4}}>
          <Tab active={1 - toPlayer} width={84}>
            <Icon name="layout-grid" size={14} color={toPlayer > 0.5 ? C.textSecondary : C.textPrimary} />
            Library
          </Tab>
          <Tab active={toPlayer} width={128 * newTab} opacity={newTab}>
            <span style={{width: 7, height: 7, borderRadius: 4, background: C.positive}} />
            Pixel-ish 6.1
            <Icon name="x" size={12} color={C.textSecondary} style={{marginLeft: 4}} />
          </Tab>
        </div>
        <div
          style={{
            position: 'absolute',
            right: 52,
            top: 12,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            opacity: toPlayer,
          }}
        >
          {rooted ? (
            <Pill tone="rootStrong" small icon="badge-check">
              Rooted · Magisk 30.7
            </Pill>
          ) : (
            <Pill tone="rootSoft" outlined small icon="loader-circle" spin={frame * 12}>
              Setting up root…
            </Pill>
          )}
          <span style={{color: C.textSecondary, font: `400 13px/1 ${FONT_MONO}`}}>emulator-5556</span>
        </div>
        <div style={{position: 'absolute', right: 16, top: 13}}>
          <Icon name="panel-right" size={18} color={C.textSecondary} />
        </div>
      </div>

      {/* Content */}
      <div style={{position: 'absolute', left: 0, top: TITLEBAR, right: 0, bottom: 0, overflow: 'hidden'}}>
        <div style={{position: 'absolute', inset: 0, opacity: 1 - toPlayer}}>
          <Sidebar plusHover={plusHover} plusPressed={plusPressed} />
          <Detail />
        </div>
        {toPlayer > 0 ? (
          <div style={{position: 'absolute', inset: 0, opacity: toPlayer}}>
            <Player screenDark={screenDark} />
          </div>
        ) : null}
        {/* Scrim */}
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: '#000',
            opacity: 0.5 * sheetVisible + 0.58 * rootIn,
          }}
        />
      </div>

      {/* New device sheet */}
      {sheetVisible > 0 ? (
        <div
          style={{
            position: 'absolute',
            left: SHEET.x,
            top: SHEET.y,
            opacity: sheetVisible,
            transform: `translateY(${-12 * (1 - sheetIn) - 10 * sheetOut}px)`,
          }}
        >
          <CreateSheet
            api35={api35}
            rowHover={rowHover}
            full={full}
            fullHover={fullHover}
            createHover={createHover}
            createPressed={createPressed}
          />
        </div>
      ) : null}

      {/* Root sheet */}
      {rootIn > 0 ? (
        <div
          style={{
            position: 'absolute',
            left: ROOT_SHEET.x,
            top: ROOT_SHEET.y,
            opacity: Math.min(1, rootIn * 2.5),
            transform: `translateY(${-12 * (1 - rootIn)}px)`,
          }}
        >
          <RootSheet frame={frame} rootedAt={rootedAt} />
        </div>
      ) : null}
    </div>
  );
};
