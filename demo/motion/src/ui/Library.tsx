import React from 'react';
import {C, FONT_MONO, FONT_UI} from '../theme';
import {Button, Icon, Kbd, Pill} from './primitives';

// Library tab: sidebar (search, Devices +, rows, Settings) and the selected device's detail.
// Layout follows crates/jidr-ui/tests/golden/launcher-populated-1280.dark.png at 1440×856.

const SIDEBAR_W = 248;

type Row = {
  name: string;
  badge: React.ReactNode;
  status: string;
};

const ROWS: Row[] = [
  {
    name: 'Phone 6.1″ FHD+',
    badge: (
      <Pill tone="rootSoft" outlined small icon="key-round">
        Full
      </Pill>
    ),
    status: 'Stopped',
  },
  {
    name: 'Tablet 11',
    badge: (
      <Pill tone="neutral" outlined small icon="terminal">
        Shell
      </Pill>
    ),
    status: 'Stopped',
  },
  {
    name: 'CI-runner-03',
    badge: (
      <span style={{display: 'flex', alignItems: 'center', gap: 4, color: C.textSecondary, font: `400 12px ${FONT_UI}`}}>
        <Icon name="minus" size={12} color={C.textSecondary} />
        No root
      </span>
    ),
    status: 'Stopped',
  },
];

export const Sidebar: React.FC<{plusHover: number; plusPressed: boolean}> = ({plusHover, plusPressed}) => (
  <div
    style={{
      position: 'absolute',
      left: 0,
      top: 0,
      width: SIDEBAR_W,
      bottom: 0,
      background: C.surfaceChrome,
      borderRight: `1px solid ${C.borderDefault}`,
      fontFamily: FONT_UI,
    }}
  >
    <div
      style={{
        position: 'absolute',
        left: 10,
        top: 12,
        width: 228,
        height: 30,
        borderRadius: 8,
        background: C.surfaceRaised,
        border: `1px solid ${C.borderDefault}`,
        boxSizing: 'border-box',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '0 8px 0 10px',
        color: C.textSecondary,
        fontSize: 13,
      }}
    >
      <Icon name="search" size={14} color={C.textSecondary} />
      <span style={{flex: 1}}>Commands and devices</span>
      <Kbd>⌘K</Kbd>
    </div>

    <div
      style={{
        position: 'absolute',
        left: 16,
        right: 10,
        top: 58,
        height: 18,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        color: C.textSecondary,
        font: `600 11px/1 ${FONT_UI}`,
      }}
    >
      <span>Devices</span>
      <div
        style={{
          width: 22,
          height: 22,
          borderRadius: 6,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: plusPressed ? C.surfaceControlHover : `rgba(39,44,52,${plusHover})`,
          transform: plusPressed ? 'scale(0.9)' : undefined,
        }}
      >
        <Icon name="plus" size={15} color={plusHover > 0.5 ? C.textPrimary : C.textSecondary} />
      </div>
    </div>

    {ROWS.map((r, i) => (
      <div
        key={r.name}
        style={{
          position: 'absolute',
          left: 8,
          width: 232,
          top: 81 + i * 54,
          height: 52,
          borderRadius: 8,
          background: i === 0 ? C.surfaceSelected : 'transparent',
          boxSizing: 'border-box',
          padding: '8px 8px',
        }}
      >
        <div style={{display: 'flex', alignItems: 'center', justifyContent: 'space-between', height: 18}}>
          <span style={{color: C.textPrimary, font: `600 13px/18px ${FONT_UI}`}}>{r.name}</span>
          {r.badge}
        </div>
        <div style={{display: 'flex', alignItems: 'center', gap: 6, marginTop: 3, color: C.textSecondary, fontSize: 12}}>
          <Icon name="square" size={11} color={C.textSecondary} />
          {r.status}
        </div>
      </div>
    ))}

    <div
      style={{
        position: 'absolute',
        left: 16,
        right: 16,
        bottom: 18,
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        color: C.textSecondary,
        fontSize: 13,
      }}
    >
      <Icon name="sliders-horizontal" size={16} color={C.textSecondary} />
      <span style={{flex: 1}}>Settings</span>
      <Kbd>⌘,</Kbd>
    </div>
  </div>
);

const KV: React.FC<{k: string; v: string; mono?: boolean}> = ({k, v, mono}) => (
  <div style={{display: 'flex', height: 18, alignItems: 'center', fontSize: 13}}>
    <span style={{width: 76, color: C.textSecondary, flex: 'none'}}>{k}</span>
    <span style={{color: C.textPrimary, fontFamily: mono ? FONT_MONO : FONT_UI, whiteSpace: 'nowrap'}}>{v}</span>
  </div>
);

const Card: React.FC<{title: string; children: React.ReactNode; footer?: React.ReactNode}> = ({
  title,
  children,
  footer,
}) => (
  <div
    style={{
      flex: 1,
      height: 152,
      borderRadius: 12,
      background: C.surfaceRaised,
      border: `1px solid ${C.borderDefault}`,
      boxSizing: 'border-box',
      padding: '14px 16px',
      position: 'relative',
    }}
  >
    <div style={{color: C.textPrimary, font: `600 13px/18px ${FONT_UI}`, marginBottom: 12}}>{title}</div>
    <div style={{display: 'flex', flexDirection: 'column', gap: 6}}>{children}</div>
    {footer ? <div style={{position: 'absolute', left: 16, bottom: 14, display: 'flex', gap: 8}}>{footer}</div> : null}
  </div>
);

/** Phone silhouette (stopped: dark screen). */
const Silhouette: React.FC = () => (
  <div
    style={{
      width: 70,
      height: 118,
      borderRadius: 14,
      background: C.bezel,
      border: `1px solid ${C.borderDefault}`,
      boxSizing: 'border-box',
      padding: 5,
    }}
  >
    <div style={{width: '100%', height: '100%', borderRadius: 9, background: C.surfaceSunken}} />
  </div>
);

export const Detail: React.FC = () => (
  <div
    style={{
      position: 'absolute',
      left: SIDEBAR_W,
      top: 0,
      right: 0,
      bottom: 0,
      background: C.surfaceBase,
      fontFamily: FONT_UI,
      padding: '28px 32px',
      boxSizing: 'border-box',
    }}
  >
    <div style={{display: 'flex', gap: 22}}>
      <Silhouette />
      <div style={{flex: 1, paddingTop: 0}}>
        <div style={{color: C.textPrimary, font: `600 32px/38px ${FONT_UI}`, letterSpacing: -0.8, marginTop: 0}}>
          Phone 6.1″ FHD+
        </div>
        <div style={{color: C.textSecondary, fontSize: 13, marginTop: 6}}>Phone 6.1″ FHD+ · 1080×2400 · 420 dpi</div>
        <div style={{display: 'flex', gap: 8, marginTop: 10, alignItems: 'center'}}>
          <Pill tone="neutral" icon="square">
            Stopped
          </Pill>
          <Pill tone="rootSoft" outlined icon="key-round">
            Full root
          </Pill>
        </div>
      </div>
      <div style={{display: 'flex', gap: 8, alignItems: 'flex-start'}}>
        <Button kind="primary" icon="play" kbd="↵">
          Start
        </Button>
        <Button icon="ellipsis" />
      </div>
    </div>

    <div style={{display: 'flex', gap: 16, marginTop: 26}}>
      <Card title="Android">
        <KV k="Version" v="Android 15 · API 35" />
        <KV k="Image" v="Google APIs" />
        <KV k="Profile" v="Phone 6.1″ FHD+" />
        <KV k="Serial" v="—" />
      </Card>
      <Card title="Hardware">
        <KV k="RAM" v="4 GB" mono />
        <KV k="CPU" v="4 cores" mono />
        <KV k="Screen" v="1080×2400 · 420 dpi" mono />
        <KV k="Disk" v="6.8 GB" mono />
      </Card>
      <Card
        title="Root health"
        footer={
          <>
            <Button small>Verify again</Button>
            <Button small kind="tertiary">
              Revert to unrooted…
            </Button>
          </>
        }
      >
        <KV k="Mode" v="Full root" />
        <KV k="Magisk" v="30.7" mono />
        <KV k="Verified" v="Not yet" />
      </Card>
    </div>

    <div style={{marginTop: 30, display: 'flex', gap: 12, alignItems: 'center'}}>
      <span style={{color: C.textPrimary, font: `600 13px/18px ${FONT_UI}`}}>Snapshots</span>
      <span style={{color: C.textSecondary, fontSize: 13}}>0</span>
    </div>
    <div style={{color: C.textSecondary, fontSize: 13, marginTop: 14}}>
      No snapshots yet. Save one from the device window with ⌘S.
    </div>

    <div style={{marginTop: 34, color: C.textPrimary, font: `600 13px/18px ${FONT_UI}`}}>Session</div>
    {[
      ['Uptime', '—'],
      ['Last started', 'Yesterday, 18:40'],
      ['Driven by', 'This window only'],
    ].map(([k, v]) => (
      <div key={k} style={{display: 'flex', fontSize: 13, marginTop: 10}}>
        <span style={{width: 132, color: C.textSecondary}}>{k}</span>
        <span style={{color: C.textPrimary}}>{v}</span>
      </div>
    ))}
  </div>
);
