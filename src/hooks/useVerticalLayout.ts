import { useSyncExternalStore } from 'react';
import { useAppStore } from '../stores/AppStore';
import { IS_MOBILE, isPortrait, onOrientationChange } from '../utils/platform';

// Vertical screens switched off: nothing to watch, never tall.
const watchNothing = () => () => {};
const neverTall = () => false;

/**
 * Whether the Vertical screens layout is on right now (Settings > Chat > Chat
 * Placement): the setting is on and the window is taller than it is wide. The
 * player then runs the full width at 16:9 and chat fills everything under it
 * (App.tsx, globals.css [data-sn-vertical]).
 *
 * It follows the window, not the monitor, so it is on for a window maximized
 * on a portrait screen and for any tall window on a wide one. Hidden chat
 * stays hidden (there is nothing to stack), and Compact View keeps its own
 * layout. Phones have their own portrait layout and never use this.
 *
 * Every surface that changes with it (the stream view, the title bar, the
 * sidebar) reads this one answer.
 */
export const useVerticalLayout = (): boolean => {
  const enabled = useAppStore((s) => !IS_MOBILE && s.settings.vertical_screens?.enabled === true);
  const applies = useAppStore((s) => !s.isTheaterMode && s.chatPlacement !== 'hidden');
  // A resize fires many times a second while a window edge is dragged; the
  // snapshot is a boolean, so a caller only re-renders when the shape crosses
  // square.
  const tall = useSyncExternalStore(enabled ? onOrientationChange : watchNothing, enabled ? isPortrait : neverTall);
  return enabled && applies && tall;
};

/** Vertical screens' "Hide title bar": on unless switched off. */
export const useVerticalHidesTitleBar = (): boolean => {
  const vertical = useVerticalLayout();
  const hide = useAppStore((s) => s.settings.vertical_screens?.hide_title_bar ?? true);
  return vertical && hide;
};

/** Vertical screens' "Hide sidebar": on unless switched off. */
export const useVerticalHidesSidebar = (): boolean => {
  const vertical = useVerticalLayout();
  const hide = useAppStore((s) => s.settings.vertical_screens?.hide_sidebar ?? true);
  return vertical && hide;
};
