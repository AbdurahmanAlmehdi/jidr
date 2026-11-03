import React from 'react';
import {interpolateColors} from 'remotion';
import {C, FONT_MONO, FONT_UI} from '../theme';
import {Button, Icon} from './primitives';

// New device sheet, laid out from crates/jidr-ui/tests/golden/create-google-apis.dark.png.
// Coordinates are sheet-relative; the sheet sits at x 380, y 44 in the 1440×900 window.

export const SHEET = {x: 380, y: 44, w: 680, h: 659};

const IMAGES = [
  {label: 'Android 16 · API 36 · Google APIs', right: 'Downloading · 1.4 / 2.1 GB', disabled: true},
  {label: 'Android 15 · API 35 · Google APIs', right: 'Installed · 2.0 GB'},
  {label: 'Android 15 · API 35 · Play Store', right: 'Download · 2.2 GB'},
  {label: 'Android 14 · API 34 · Google APIs', right: 'Installed · 1.9 GB'},
  {label: 'Android 13 · API 33 · Play Store', right: 'Installed · 1.8 GB'},
  {label: 'Android 11 · API 30 · AOSP', right: 'Installed · 1.2 GB'},
];

const Label: React.FC<{x: number; y: number; children: React.ReactNode; right?: boolean; w?: number}> = ({
  x,
  y,
  children,
  right,
  w,
}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      textAlign: right ? 'right' : 'left',
      color: right ? C.textSecondary : C.textPrimary,
      font: `${right ? 400 : 500} ${right ? 11.5 : 12.5}px/16px ${FONT_UI}`,
      opacity: right ? 0.9 : 0.85,
    }}
  >
    {children}
  </div>
);

const Radio: React.FC<{on: number; color?: string}> = ({on, color = C.accentFill}) => (
  <span
    style={{
      width: 16,
      height: 16,
      borderRadius: 8,
      boxSizing: 'border-box',
      border: `${1 + on * 4}px solid ${interpolateColors(on, [0, 1], [C.borderStrong, color])}`,
      background: interpolateColors(on, [0, 1], [C.surfaceControl, '#ffffff']),
      flex: 'none',
    }}
  />
);

const Field: React.FC<{
  x: number;
  y: number;
  w: number;
  value: string;
  unit?: string;
  mono?: boolean;
  focus?: boolean;
  chevron?: boolean;
}> = ({x, y, w, value, unit, mono, focus, chevron}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      height: 30,
      borderRadius: 8,
      boxSizing: 'border-box',
      background: C.surfaceSunken,
      border: `1px solid ${focus ? C.accentText : C.borderDefault}`,
      boxShadow: focus ? `0 0 0 1px ${C.accentText}` : undefined,
      display: 'flex',
      alignItems: 'center',
      padding: '0 10px',
      gap: 8,
      color: C.textPrimary,
      font: `400 13px/1 ${mono ? FONT_MONO : FONT_UI}`,
    }}
  >
    <span style={{flex: 1}}>{value}</span>
    {unit ? <span style={{color: C.textTertiary, fontFamily: FONT_MONO, fontSize: 11.5}}>{unit}</span> : null}
    {chevron ? <Icon name="chevron-down" size={14} color={C.textSecondary} /> : null}
  </div>
);

type ModeCardProps = {
  x: number;
  title: string;
  body: string;
  icon: string;
  on: number;
  featured?: boolean;
  hover?: number;
};

const ModeCard: React.FC<ModeCardProps> = ({x, title, body, icon, on, featured, hover = 0}) => {
  const baseBg = featured ? C.rootSoftContainer : C.surfaceRaised;
  const baseBd = featured ? C.rootSoftBorder : hover > 0.5 ? C.borderStrong : C.borderDefault;
  const bg = interpolateColors(on, [0, 1], [baseBg, C.accentContainer]);
  const bd = interpolateColors(on, [0, 1], [baseBd, C.accentFill]);
  const ink = interpolateColors(on, [0, 1], [featured ? C.root : C.textSecondary, C.accentText]);
  const bodyInk = interpolateColors(on, [0, 1], [featured ? C.rootSoftOn : C.textSecondary, C.textSecondary]);
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: 393,
        width: 206,
        height: 95,
        borderRadius: 12,
        boxSizing: 'border-box',
        background: bg,
        border: `1px solid ${bd}`,
        boxShadow: on > 0 ? `0 0 0 ${on}px ${C.accentFill}` : undefined,
        padding: 14,
        transform: `scale(${1 + hover * 0.012})`,
      }}
    >
      <div style={{display: 'flex', alignItems: 'center', gap: 7}}>
        <Icon name={icon} size={16} color={ink} />
        <span style={{flex: 1, color: C.textPrimary, font: `600 13px/18px ${FONT_UI}`}}>{title}</span>
        <Radio on={on} />
      </div>
      <div style={{marginTop: 6, color: bodyInk, font: `400 12px/17px ${FONT_UI}`}}>{body}</div>
    </div>
  );
};

export const CreateSheet: React.FC<{
  api35: number; // 0 → API 34 selected, 1 → API 35 selected
  rowHover: number;
  full: number; // 0 → Shell selected, 1 → Full root selected
  fullHover: number;
  createHover: number;
  createPressed: boolean;
}> = ({api35, rowHover, full, fullHover, createHover, createPressed}) => {
  const selBg = (i: number) => {
    if (i === 1) return `rgba(27,42,77,${api35})`;
    if (i === 3) return `rgba(27,42,77,${1 - api35})`;
    return 'transparent';
  };
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        top: 0,
        width: SHEET.w,
        height: SHEET.h,
        background: C.elev3,
        border: `1px solid ${C.borderDefault}`,
        borderTop: 'none',
        borderRadius: '0 0 14px 14px',
        overflow: 'hidden',
        fontFamily: FONT_UI,
        boxShadow: '0 24px 60px rgba(0,0,0,0.55)',
      }}
    >
      <div style={{position: 'absolute', left: 20, top: 19, color: C.textPrimary, font: `600 15px/20px ${FONT_UI}`}}>
        New device
      </div>
      <div style={{position: 'absolute', left: 20, top: 42, color: C.textSecondary, font: `400 12px/16px ${FONT_UI}`}}>
        Defaults are filled in. Change what you need, then create.
      </div>

      <Label x={20} y={76}>
        Name
      </Label>
      <Field x={20} y={97} w={312} value="Pixel-ish 6.1" focus />
      <Label x={347} y={76}>
        Hardware profile
      </Label>
      <Field x={347} y={97} w={313} value="Phone 6.1″ FHD+ · 1080×2400" chevron />

      <Label x={20} y={141}>
        Android version
      </Label>
      <Label x={300} y={142} w={360} right>
        Only images that run on Apple Silicon (arm64) are listed.
      </Label>
      <div
        style={{
          position: 'absolute',
          left: 20,
          top: 163,
          width: 640,
          height: 194,
          borderRadius: 8,
          border: `1px solid ${C.borderDefault}`,
          background: C.surfaceSunken,
          overflow: 'hidden',
          boxSizing: 'border-box',
        }}
      >
        {IMAGES.map((img, i) => (
          <div
            key={img.label}
            style={{
              height: 32,
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: '0 10px',
              background: i === 1 && rowHover > 0 && api35 < 1 ? `rgba(39,44,52,${rowHover})` : selBg(i),
              color: img.disabled ? C.textDisabled : C.textPrimary,
              fontSize: 13,
            }}
          >
            {img.disabled ? (
              <Icon name="download" size={16} color={C.textDisabled} />
            ) : (
              <Radio on={i === 1 ? api35 : i === 3 ? 1 - api35 : 0} />
            )}
            <span style={{flex: 1}}>{img.label}</span>
            <span style={{color: C.textSecondary, fontSize: 12}}>{img.right}</span>
          </div>
        ))}
      </div>

      <Label x={20} y={372}>
        Root mode
      </Label>
      <ModeCard
        x={20}
        title="None"
        icon="minus"
        body="A stock device. Test apps as users see them."
        on={0}
      />
      <ModeCard
        x={237}
        title="Shell root (adb)"
        icon="terminal"
        body="adb root works. Nothing on the system changes."
        on={1 - full}
      />
      <ModeCard
        x={454}
        title="Full root"
        icon="key-round"
        body="Installs and verifies Magisk 30.7. Reboots once (about 40 s)."
        on={full}
        featured
        hover={fullHover * (1 - full)}
      />

      <Label x={20} y={507}>
        RAM
      </Label>
      <Field x={20} y={528} w={204} value="4" unit="GB" mono />
      <Label x={238} y={507}>
        CPU cores
      </Label>
      <Field x={238} y={528} w={204} value="4" unit="of 10" mono />
      <Label x={456} y={507}>
        Screen
      </Label>
      <Field x={456} y={528} w={204} value="1080×2400" unit="420 dpi" mono />
      <div style={{position: 'absolute', left: 20, top: 563, color: C.textSecondary, fontSize: 11.5}}>
        Your Mac has 16 GB.
      </div>
      <div style={{position: 'absolute', left: 456, top: 563, color: C.textSecondary, fontSize: 11.5}}>
        From the profile.
      </div>

      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          bottom: 0,
          height: 54,
          background: C.surfaceChrome,
          borderTop: `1px solid ${C.borderDefault}`,
          display: 'flex',
          alignItems: 'center',
          padding: '0 20px',
          gap: 8,
        }}
      >
        <span
          style={{
            width: 16,
            height: 16,
            borderRadius: 4,
            background: C.accentFill,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <Icon name="check" size={12} color="#fff" stroke={2.5} />
        </span>
        <span style={{color: C.textPrimary, fontSize: 13, marginLeft: 2, flex: 1}}>Start after creating</span>
        <Button>Cancel</Button>
        <Button kind="primary" kbd="⌘↵" hover={createHover > 0.5} pressed={createPressed}>
          Create
        </Button>
      </div>
    </div>
  );
};

