// Keeps the watched stream's volatile stats fresh.
//
// `currentStream` is written once when playback starts and then never updated,
// except by the `eventsub://channel-update` listener, which carries title,
// category and game id — and nothing else. So the viewer count shown while
// watching is whatever it happened to be the instant the stream was opened, for
// the entire session. Both shells read the same frozen field.
//
// Polling on `useVisibleInterval` rather than a bare setInterval is what makes
// this correct rather than merely present: it skips ticks while the window is
// hidden, and fires immediately when it comes back, so returning to the app
// shows a current number instead of a stale one until the next tick.
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { useVisibleInterval } from './useVisibleInterval';
import type { TwitchStream } from '../types';

// Matches the MultiChat viewer counter's cadence. Viewer counts move slowly
// enough that a faster poll buys nothing, and this is one Helix call per
// stream per interval.
const STATS_POLL_MS = 45_000;

export function useCurrentStreamStats(): void {
  const login = useAppStore((s) => s.currentStream?.user_login);
  const mediaType = useAppStore((s) => s.currentMediaType);

  useVisibleInterval(async () => {
    // Live streams only. A clip or VOD has no live viewer count, and its
    // `view_count` is a total that does not move while you watch it.
    if (!login || mediaType !== 'live') return;
    try {
      const fresh = await invoke<TwitchStream | null>('check_stream_online', {
        userLogin: login,
      });
      if (!fresh) return;

      const current = useAppStore.getState().currentStream;
      // Re-read rather than closing over it: the user may have switched
      // channels while this request was in flight, and writing then would
      // stamp one channel's viewer count onto another.
      if (!current || current.user_login !== login) return;
      if (current.viewer_count === fresh.viewer_count) return;

      // Merged, not replaced. `check_stream_online` returns the Helix streams
      // shape, which carries no avatar and no broadcaster type, so assigning it
      // wholesale would blank fields the UI is already showing.
      useAppStore.getState().setCurrentStream({
        ...current,
        viewer_count: fresh.viewer_count,
      });
    } catch {
      // Offline or a transient failure; the next tick retries.
    }
  }, STATS_POLL_MS);
}
