// Chat drawn straight over the video (SN-40): the tall layout's lower third and
// the landscape overlay column. Brandon's pick (2026-11-10): rows over the
// picture always get a dark soft backing and light text, like TikTok and
// Twitch, while the rest of the app keeps its theme. A dark theme draws there
// exactly as everywhere else; a light theme draws that chat in the app's
// default dark theme, Frosted Glass (the iPhone uses the same palette,
// `Palette.frostedGlass`).
//
// How: the same color variables applyTheme paints on <html>, set again on the
// element that holds that chat (with the `sn-over-video` class, for the few
// light-theme rules that must not reach it), so every row, backing, tab and pill
// inside it resolves the dark values with no per-row work. Names are adjusted
// in script, so the pane also tells useNameColorAdjust the background is dark
// (NameColorToneContext).
import { useMemo, useSyncExternalStore, type CSSProperties } from 'react';
import {
  DEFAULT_THEME_ID,
  getThemeById,
  glassSurfaces,
  isThemeDark,
  phoneFloors,
  subscribeTheme,
  themeVersion,
} from '../../themes';

/** The class on the element whose chat is over the video. */
export const OVER_VIDEO_CLASS = 'sn-over-video';

const lighten = (hex: string, percent: number): string => {
  const h = hex.replace('#', '');
  const up = (i: number) => {
    const c = parseInt(h.slice(i, i + 2), 16);
    return Math.min(255, Math.round(c + (255 - c) * (percent / 100)))
      .toString(16)
      .padStart(2, '0');
  };
  return `#${up(0)}${up(2)}${up(4)}`;
};

const rgbChannels = (hex: string): string => {
  const h = hex.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)).join(', ');
};

/**
 * The color variables chat over video takes in a light theme: Frosted Glass's,
 * its floors flattened for the phone and its surfaces at the glassiness on
 * screen now (Glassiness, the blur switch), as applyTheme paints a theme.
 */
export const overVideoVars = (): Record<string, string> => {
  const theme = getThemeById(DEFAULT_THEME_ID);
  if (!theme) return {};
  const p = theme.palette;
  const floors = phoneFloors(p.background, p.backgroundTertiary);
  const surfaces = glassSurfaces(p, floors.tertiary);
  return {
    '--color-background': floors.background,
    '--color-background-secondary': p.backgroundSecondary,
    '--color-background-tertiary': floors.tertiary,
    '--color-surface': surfaces.surface,
    '--color-surface-hover': surfaces.surfaceHover,
    '--color-surface-active': surfaces.surfaceActive,
    '--color-text-primary': p.textPrimary,
    '--color-text-secondary': p.textSecondary,
    '--color-text-muted': p.textMuted,
    '--color-accent': p.accent,
    '--color-accent-hover': p.accentHover,
    '--color-accent-muted': p.accentMuted,
    '--color-accent-on-surface': p.accentOnSurface ?? p.accent,
    '--color-accent-neon': lighten(p.accent, 30),
    '--color-accent-rgb': rgbChannels(p.accent),
    '--color-border': p.border,
    '--color-border-light': p.borderLight,
    '--color-border-subtle': p.borderSubtle,
    '--color-success': p.success,
    '--color-warning': p.warning,
    '--color-error': p.error,
    '--color-info': p.info,
    '--color-scrollbar-thumb': p.scrollbarThumb,
    '--color-scrollbar-track': p.scrollbarTrack,
    '--glass-opacity': p.glassOpacity,
    '--glass-hover-opacity': p.glassHoverOpacity,
    '--glass-active-opacity': p.glassActiveOpacity,
    '--color-highlight-pink': p.highlight.pink,
    '--color-highlight-purple': p.highlight.purple,
    '--color-highlight-blue': p.highlight.blue,
    '--color-highlight-cyan': p.highlight.cyan,
    '--color-highlight-green': p.highlight.green,
    '--color-highlight-yellow': p.highlight.yellow,
    '--color-highlight-orange': p.highlight.orange,
    '--color-highlight-red': p.highlight.red,
  };
};

/** The style for chat over video: nothing for a dark theme (it looks exactly as
 *  it does everywhere), Frosted Glass's variables for a light one. */
export const overVideoStyle = (themeDark: boolean): CSSProperties | undefined =>
  themeDark ? undefined : (overVideoVars() as CSSProperties);

/** The applied theme's tone, following every theme change. */
export const useThemeDark = (): boolean => useSyncExternalStore(subscribeTheme, isThemeDark, isThemeDark);

/** `overVideoStyle` for chat that is over video right now (`on`), else nothing;
 *  built again only when the painted theme changes (a theme, Glassiness, blur). */
export const useOverVideoStyle = (on: boolean): CSSProperties | undefined => {
  const dark = useThemeDark();
  const version = useSyncExternalStore(subscribeTheme, themeVersion, themeVersion);
  // `version` is the cache key: the variables read the glass state it stands for.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => (on ? overVideoStyle(dark) : undefined), [on, dark, version]);
};
