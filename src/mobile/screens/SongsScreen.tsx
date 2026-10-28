// You > Songs: Your songs as a screen of its own, the same body the desktop's
// panel draws. It sits over the watch layer, so a song card in a chat opens it
// without closing the stream; back steps from a song to the list to out.
import React, { lazy, Suspense, useEffect } from 'react';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { registerSongsSurface } from '../../utils/songHistory';

const SongsPanel = lazy(() => import('../../components/songs/SongsPanel'));

export const SongsScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.songsOpen);
  const focus = useMobileNavStore((s) => s.songsFocus);
  const closeSongs = useMobileNavStore((s) => s.closeSongs);
  const setSongsFocus = useMobileNavStore((s) => s.setSongsFocus);

  useEffect(() => registerSongsSurface('screen'), []);

  // Opening from a chat leaves the composer's keyboard up; the screen covers it.
  useEffect(() => {
    if (open) (document.activeElement as HTMLElement | null)?.blur?.();
  }, [open]);

  return (
    <DrillInScreen
      open={open}
      layerKey="songs"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      {open && (
        <Suspense fallback={null}>
          <SongsPanel variant="screen" focus={focus} onFocus={setSongsFocus} onClose={closeSongs} />
        </Suspense>
      )}
    </DrillInScreen>
  );
};
