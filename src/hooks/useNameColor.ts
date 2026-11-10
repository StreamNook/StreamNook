// Readable name colors, bound to the setting and the theme.
//
// Every surface that paints a chatter's name in their own color goes through
// one of these two, so the adjustment is one decision made in one place. A
// 7TV paint is not a color and is never touched: it comes in as a style, not
// a hex string, and the helper leaves anything that is not hex alone.
import { createContext, useCallback, useContext } from 'react';
import { useAppStore } from '../stores/AppStore';
import { isThemeDark } from '../themes';
import { adjustNameColor, type NameColorAdjustment } from '../utils/nameColor';

const DEFAULT_MODE: NameColorAdjustment = 'hsl_loop';

const modeOf = (raw: unknown): NameColorAdjustment => (raw === 'off' ? 'off' : DEFAULT_MODE);

/**
 * What the names below sit on, where it is not the theme's background: `true`
 * for a dark one (the phones' chat over video, which takes dark ink in every
 * theme), `false` for a light one, `null` (the default) to follow the theme.
 */
export const NameColorToneContext = createContext<boolean | null>(null);

/** For components: re-renders when the setting changes, or what the names sit on. */
export function useNameColorAdjust(): (color: string | undefined | null) => string | undefined {
  const mode = useAppStore((s) => modeOf(s.settings.chat_design?.name_color_adjustment));
  const tone = useContext(NameColorToneContext);
  return useCallback(
    (color) => (color ? adjustNameColor(color, mode, tone ?? isThemeDark()) : undefined),
    [mode, tone],
  );
}

/** For code outside React (store reads, event handlers). */
export function readableNameColor(color: string): string {
  return adjustNameColor(color, modeOf(useAppStore.getState().settings.chat_design?.name_color_adjustment), isThemeDark());
}
