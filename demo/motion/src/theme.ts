// Dark-theme values from docs/design/tokens.json (names kept, prefix dropped).
import {loadFont} from '@remotion/fonts';
import geist400 from '@fontsource/geist/files/geist-latin-400-normal.woff2';
import geist500 from '@fontsource/geist/files/geist-latin-500-normal.woff2';
import geist600 from '@fontsource/geist/files/geist-latin-600-normal.woff2';
import geist700 from '@fontsource/geist/files/geist-latin-700-normal.woff2';
import mono400 from '@fontsource/geist-mono/files/geist-mono-latin-400-normal.woff2';
import mono500 from '@fontsource/geist-mono/files/geist-mono-latin-500-normal.woff2';
import kufi600 from '@fontsource/noto-kufi-arabic/files/noto-kufi-arabic-arabic-600-normal.woff2';

export const C = {
  surfaceBase: '#0e1014',
  surfaceChrome: '#13161b',
  surfaceRaised: '#191c22',
  surfaceSunken: '#0a0c0f',
  surfaceControl: '#20242b',
  surfaceControlHover: '#272c34',
  surfaceSelected: '#232936',
  bezel: '#1c1f25',
  elev2: '#22262e',
  elev3: '#252a33',
  textPrimary: '#e7e9ee',
  textSecondary: '#9ba3b0',
  textTertiary: '#8f97a5',
  textDisabled: '#5a616d',
  textCode: '#b9c8e8',
  borderSubtle: '#1e2229',
  borderDefault: '#262b33',
  borderStrong: '#6e7785',
  accentFill: '#3d6bf0',
  accentHover: '#3462e6',
  accentPressed: '#2b55cc',
  accentText: '#7aa2ff',
  accentContainer: '#1b2a4d',
  positive: '#5dd38a',
  warning: '#f2b94b',
  info: '#7aa2ff',
  root: '#a78bfa',
  scrim: 'rgba(0,0,0,0.64)',
  neutralContainer: '#20242b',
  neutralOn: '#b8bfca',
  neutralBorder: '#3a414c',
  positiveContainer: '#133222',
  positiveOn: '#74e0a0',
  positiveBorder: '#23573a',
  transitionContainer: '#17233f',
  transitionOn: '#93b4ff',
  transitionBorder: '#2c4580',
  warningContainer: '#3a2c0f',
  warningOn: '#f5c86b',
  warningBorder: '#6b5118',
  rootSoftContainer: '#241d3d',
  rootSoftOn: '#c3b1ff',
  rootSoftBorder: '#7e69cc',
  rootStrong: '#a78bfa',
  rootStrongOn: '#120d22',
  // assets/brand/README.md
  brandBlueTop: '#2A88FA',
  brandBlueBottom: '#1267E0',
  lilac: '#B9A6F7',
};

export const FONT_UI = 'Geist, -apple-system, system-ui, sans-serif';
export const FONT_MONO = '"Geist Mono", ui-monospace, Menlo, monospace';
export const FONT_AR = '"Noto Kufi Arabic", sans-serif';

const faces: Array<[string, string, string]> = [
  ['Geist', geist400, '400'],
  ['Geist', geist500, '500'],
  ['Geist', geist600, '600'],
  ['Geist', geist700, '700'],
  ['Geist Mono', mono400, '400'],
  ['Geist Mono', mono500, '500'],
  ['Noto Kufi Arabic', kufi600, '600'],
];

for (const [family, url, weight] of faces) {
  loadFont({family, url, weight, format: 'woff2'});
}
