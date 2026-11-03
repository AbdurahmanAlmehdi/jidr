import React from 'react';
import {C, FONT_MONO, FONT_UI} from '../theme';
import {Icon} from './primitives';

// Device tab behind the Root sheet, after crates/jidr-ui/tests/golden/root-progress.dark.png
// (window content area, 1440×856). The sheet's scrim dims all of it.

const RAIL = ['smartphone', 'smartphone', 'smartphone', 'smartphone'];
const TOOLS = ['arrow-left', 'circle', 'square', '|', 'volume-2', 'volume-1', '|', 'rotate-ccw', 'rotate-cw', 'power', '|', 'camera', 'circle-dot', 'scan', 'clipboard'];

/** The phone's screen: a light app, black while it reboots. */
const Screen: React.FC<{dark: number}> = ({dark}) => (
  <div style={{position: 'absolute', inset: 0, background: '#f5f5f2', overflow: 'hidden'}}>
    <div style={{position: 'absolute', left: 14, top: 22, width: 90, height: 7, borderRadius: 4, background: '#c9c9c4'}} />
    <div style={{position: 'absolute', left: 14, top: 40, width: 190, height: 14, borderRadius: 4, background: '#2b2b2b'}} />
    <div style={{position: 'absolute', left: 14, top: 72, right: 14, height: 180, borderRadius: 12, background: '#e8e5dc'}} />
    <div style={{position: 'absolute', left: 14, top: 264, right: 14, height: 62, borderRadius: 12, background: '#ffffff'}} />
    <div style={{position: 'absolute', left: 14, top: 338, right: 14, height: 36, borderRadius: 18, background: '#1d4fbf'}} />
    {[392, 434].map((t) => (
      <div key={t} style={{position: 'absolute', left: 14, right: 14, top: t, height: 30, borderBottom: '1px solid #dcdcd6'}} />
    ))}
    <div style={{position: 'absolute', left: '50%', bottom: 10, width: 98, height: 4, marginLeft: -49, borderRadius: 2, background: '#222'}} />
    <div style={{position: 'absolute', inset: 0, background: '#000', opacity: dark}} />
  </div>
);

export const Player: React.FC<{screenDark: number}> = ({screenDark}) => (
  <div style={{position: 'absolute', inset: 0, fontFamily: FONT_UI, background: '#0a0b0d'}}>
    {/* Device rail */}
    <div style={{position: 'absolute', left: 0, top: 0, bottom: 0, width: 52, background: C.surfaceChrome, borderRight: `1px solid ${C.borderDefault}`}}>
      {RAIL.map((n, i) => (
        <div
          key={i}
          style={{
            position: 'absolute',
            left: 10,
            top: 11 + i * 38,
            width: 32,
            height: 32,
            borderRadius: 8,
            background: i === 0 ? C.surfaceSelected : 'transparent',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <Icon name={n} size={16} color={i === 0 ? C.accentText : C.textSecondary} />
        </div>
      ))}
    </div>

    {/* Stage with the phone */}
    <div style={{position: 'absolute', left: 52, top: 0, width: 1020, bottom: 34}}>
      <div
        style={{
          position: 'absolute',
          left: 330,
          top: 34,
          width: 360,
          height: 764,
          borderRadius: 34,
          background: C.bezel,
          border: `1px solid ${C.borderDefault}`,
          boxSizing: 'border-box',
          padding: 10,
        }}
      >
        <div style={{position: 'relative', width: '100%', height: '100%', borderRadius: 24, overflow: 'hidden'}}>
          <Screen dark={screenDark} />
        </div>
      </div>
    </div>

    {/* Logcat bar */}
    <div
      style={{
        position: 'absolute',
        left: 52,
        width: 1020,
        bottom: 0,
        height: 34,
        borderTop: `1px solid ${C.borderDefault}`,
        background: C.surfaceChrome,
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '0 14px',
        boxSizing: 'border-box',
        fontSize: 13,
        color: C.textSecondary,
      }}
    >
      <Icon name="scroll-text" size={15} color={C.textSecondary} />
      <span style={{color: C.textPrimary, fontWeight: 600}}>Logcat</span>
      <span style={{fontFamily: FONT_MONO, fontSize: 12, color: C.textTertiary}}>I Magisk  Magisk 30.7 (30700)</span>
    </div>

    {/* Toolbar */}
    <div style={{position: 'absolute', left: 1072, top: 0, bottom: 0, width: 48, background: C.surfaceChrome, borderLeft: `1px solid ${C.borderDefault}`}}>
      {(() => {
        let y = 12;
        return TOOLS.map((n, i) => {
          if (n === '|') {
            const el = <div key={i} style={{position: 'absolute', left: 12, top: y + 4, width: 24, height: 1, background: C.borderDefault}} />;
            y += 10;
            return el;
          }
          const el = (
            <div key={i} style={{position: 'absolute', left: 14, top: y}}>
              <Icon name={n} size={20} color={C.textSecondary} />
            </div>
          );
          y += 36;
          return el;
        });
      })()}
    </div>

    {/* Widget panel */}
    <div style={{position: 'absolute', left: 1120, top: 0, right: 0, bottom: 0, background: C.surfaceChrome, padding: 14, boxSizing: 'border-box'}}>
      <div
        style={{
          height: 30,
          borderRadius: 8,
          border: `1px solid ${C.borderDefault}`,
          display: 'flex',
          alignItems: 'center',
          gap: 4,
          padding: 2,
          boxSizing: 'border-box',
          fontSize: 12.5,
          color: C.textSecondary,
        }}
      >
        {['Battery', 'GPS', 'Network', 'Sensors'].map((t, i) => (
          <span
            key={t}
            style={{
              padding: '0 10px',
              height: 24,
              lineHeight: '24px',
              borderRadius: 6,
              background: i === 0 ? C.surfaceControl : 'transparent',
              color: i === 0 ? C.textPrimary : C.textSecondary,
            }}
          >
            {t}
          </span>
        ))}
      </div>
      <div style={{marginTop: 26, color: C.textSecondary, fontSize: 12, fontWeight: 600}}>Battery level</div>
      <div style={{marginTop: 10, display: 'flex', alignItems: 'baseline', gap: 10}}>
        <span style={{color: C.textPrimary, fontSize: 21}}>64%</span>
        <span style={{fontSize: 11.5, color: C.textSecondary}}>Discharging</span>
      </div>
      <div style={{marginTop: 16, height: 4, borderRadius: 2, background: '#000', position: 'relative', width: 204}}>
        <div style={{position: 'absolute', left: 0, top: 0, height: 4, width: 130, borderRadius: 2, background: C.accentFill}} />
        <div style={{position: 'absolute', left: 122, top: -6, width: 16, height: 16, borderRadius: 8, background: '#fff'}} />
      </div>
      <div style={{marginTop: 34, display: 'flex', justifyContent: 'space-between', color: C.textPrimary, fontSize: 13}}>
        Charging
        <span style={{width: 32, height: 18, borderRadius: 9, background: C.surfaceControl, border: `1px solid ${C.borderDefault}`}} />
      </div>
    </div>
  </div>
);
