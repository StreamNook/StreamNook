import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { IS_MOBILE } from '../utils/platform';
import {
  pageOverPlayerOpen,
  titleBarAutoHides,
  titleBarLifted,
  titleBarTuckMode,
  watchingForTitleBar,
} from '../utils/titleBarTuck';
import { useStreamOnlyFullscreen } from './useStreamOnlyFullscreen';

/**
 * Whether the title bar is in its tucking layout: stream-only full screen, or
 * "Title bar auto-hide" with a stream, VOD or MultiNook grid on screen
 * (`watchingForTitleBar`). While true, nothing reserves the bar's 40px and
 * the bar slides away until the pointer reaches the top edge.
 *
 * Every surface that clears the bar (the stream area, the sidebar, the bar
 * itself) reads this one answer, so they can never disagree about where the
 * top of the content is.
 */
export const useTitleBarAutoHide = (): boolean => {
  const streamOnlyFullscreen = useStreamOnlyFullscreen();
  const isMultiNookActive = usemultiNookStore((s) => s.isMultiNookActive);
  const windowFullscreen = useAppStore((s) => s.isWindowFullscreen);
  const hideWhileWatching = useAppStore(
    (s) => s.settings.video_player?.hide_title_bar_while_watching === true,
  );
  const watching = useAppStore((s) =>
    watchingForTitleBar({ isHomeActive: s.isHomeActive, streamUrl: s.streamUrl, isMultiNookActive }),
  );
  if (IS_MOBILE) return false;
  return titleBarAutoHides({ hideWhileWatching, watching, windowFullscreen, streamOnlyFullscreen });
};

/**
 * Whether the title bar draws above the player right now (`titleBarLifted`).
 * The bar and the Island's surface, which grows out of the bar's door, read
 * this one answer so they always lift together. `ownPageOpen` is a page the
 * caller opens itself (the bar's About card and update dialog), which keeps
 * the bar under it like any other page.
 */
export const useTitleBarLifted = (ownPageOpen = false): boolean => {
  const layoutAutoHides = useTitleBarAutoHide();
  const playerFullscreen = useAppStore((s) => s.isPlayerFullscreen);
  const pageOpen = useAppStore((s) => pageOverPlayerOpen(s));
  return titleBarLifted({
    tuckMode: titleBarTuckMode({ layoutAutoHides, playerFullscreen }),
    playerFullscreen,
    pageOpen: pageOpen || ownPageOpen,
  });
};
