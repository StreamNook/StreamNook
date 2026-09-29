import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';

/**
 * Full screen while watching shows only the stream and its chat: the title bar
 * and the sidebar tuck away and come back when the cursor reaches their edge.
 *
 * Scoped to the watch view. Home keeps its chrome in full screen, because its
 * tab strip and search live in the title bar and the grid has nothing to gain
 * from the space. The player's own fullscreen (Plyr) never sets
 * isWindowFullscreen, so it is unaffected.
 */
export const useStreamOnlyFullscreen = (): boolean => {
  const isMultiNookActive = usemultiNookStore((s) => s.isMultiNookActive);
  return useAppStore(
    (s) =>
      s.isWindowFullscreen &&
      s.settings.fullscreen_stream_only !== false &&
      !s.isHomeActive &&
      (!!s.streamUrl || isMultiNookActive),
  );
};
