import React from 'react';
import {ICONS} from '../icons';
import {C, FONT_MONO, FONT_UI} from '../theme';

export const Icon: React.FC<{
  name: string;
  size?: number;
  color?: string;
  stroke?: number;
  style?: React.CSSProperties;
}> = ({name, size = 16, color = 'currentColor', stroke = 1.75, style}) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke={color}
    strokeWidth={stroke}
    strokeLinecap="round"
    strokeLinejoin="round"
    style={{flex: 'none', display: 'block', color, ...style}}
    dangerouslySetInnerHTML={{__html: ICONS[name] ?? ''}}
  />
);

export const Kbd: React.FC<{children: React.ReactNode; onAccent?: boolean}> = ({children, onAccent}) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      minWidth: 18,
      height: 18,
      padding: '0 4px',
      borderRadius: 4,
      border: `1px solid ${onAccent ? 'rgba(255,255,255,0.75)' : C.borderDefault}`,
      color: onAccent ? 'rgba(255,255,255,0.85)' : C.textTertiary,
      font: `500 11px/1 ${FONT_UI}`,
      boxSizing: 'border-box',
    }}
  >
    {children}
  </span>
);

type BtnKind = 'primary' | 'secondary' | 'tertiary';

export const Button: React.FC<{
  kind?: BtnKind;
  icon?: string;
  kbd?: string;
  small?: boolean;
  pressed?: boolean;
  hover?: boolean;
  children?: React.ReactNode;
  style?: React.CSSProperties;
}> = ({kind = 'secondary', icon, kbd, small, pressed, hover, children, style}) => {
  const bg =
    kind === 'primary'
      ? pressed
        ? C.accentPressed
        : hover
          ? C.accentHover
          : C.accentFill
      : kind === 'tertiary'
        ? hover
          ? C.surfaceControl
          : 'transparent'
        : pressed
          ? '#2e343e'
          : hover
            ? C.surfaceControlHover
            : C.surfaceControl;
  const bd = kind === 'primary' ? bg : kind === 'tertiary' ? 'transparent' : C.borderDefault;
  const fg = kind === 'primary' ? '#fff' : kind === 'tertiary' ? C.textSecondary : C.textPrimary;
  return (
    <div
      style={{
        height: small ? 24 : 30,
        padding: children ? (small ? '0 8px' : '0 12px') : 0,
        width: children ? undefined : small ? 24 : 30,
        borderRadius: small ? 6 : 8,
        background: bg,
        border: `1px solid ${bd}`,
        color: fg,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        gap: small ? 6 : 8,
        font: `500 ${small ? 12 : 13}px/1 ${FONT_UI}`,
        boxSizing: 'border-box',
        whiteSpace: 'nowrap',
        transform: pressed ? 'scale(0.97)' : undefined,
        ...style,
      }}
    >
      {icon ? <Icon name={icon} size={small ? 14 : 16} color={fg} /> : null}
      {children}
      {kbd ? <Kbd onAccent={kind === 'primary'}>{kbd}</Kbd> : null}
    </div>
  );
};

type Tone = 'neutral' | 'positive' | 'transition' | 'warning' | 'rootSoft' | 'rootStrong';

const TONES: Record<Tone, {bg: string; fg: string; bd: string}> = {
  neutral: {bg: C.neutralContainer, fg: C.neutralOn, bd: C.neutralBorder},
  positive: {bg: C.positiveContainer, fg: C.positiveOn, bd: C.positiveBorder},
  transition: {bg: C.transitionContainer, fg: C.transitionOn, bd: C.transitionBorder},
  warning: {bg: C.warningContainer, fg: C.warningOn, bd: C.warningBorder},
  rootSoft: {bg: C.rootSoftContainer, fg: C.rootSoftOn, bd: C.rootSoftBorder},
  rootStrong: {bg: C.rootStrong, fg: C.rootStrongOn, bd: C.rootStrong},
};

/** StatusPill / RootBadge: 24 px default, 20 px small. */
export const Pill: React.FC<{
  tone: Tone;
  icon?: string;
  outlined?: boolean;
  small?: boolean;
  dot?: string;
  spin?: number;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({tone, icon, outlined, small, dot, spin, children, style}) => {
  const t = TONES[tone];
  return (
    <div
      style={{
        height: small ? 20 : 24,
        padding: small ? '0 8px' : '0 10px',
        borderRadius: 999,
        background: outlined ? (tone === 'rootSoft' ? t.bg : 'transparent') : t.bg,
        border: `1px solid ${outlined ? t.bd : t.bg}`,
        color: t.fg,
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        font: `500 ${small ? 11.5 : 13}px/1 ${FONT_UI}`,
        boxSizing: 'border-box',
        whiteSpace: 'nowrap',
        ...style,
      }}
    >
      {dot ? <span style={{width: 7, height: 7, borderRadius: 4, background: dot}} /> : null}
      {icon ? (
        <Icon
          name={icon}
          size={small ? 12 : 14}
          color={t.fg}
          style={spin !== undefined ? {transform: `rotate(${spin}deg)`} : undefined}
        />
      ) : null}
      {children}
    </div>
  );
};

export const Mono: React.FC<{children: React.ReactNode; style?: React.CSSProperties}> = ({children, style}) => (
  <span style={{fontFamily: FONT_MONO, ...style}}>{children}</span>
);

export const TrafficLights: React.FC<{y?: number; x?: number}> = ({x = 14, y = 15}) => (
  <div style={{position: 'absolute', left: x, top: y, display: 'flex', gap: 8}}>
    {['#ff5f57', '#febc2e', '#28c840'].map((c) => (
      <span key={c} style={{width: 12, height: 12, borderRadius: 6, background: c}} />
    ))}
  </div>
);
