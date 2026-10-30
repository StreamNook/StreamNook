import { useState } from 'react';
import { useChatConnectionStore } from '../stores/chatConnectionStore';

/**
 * How many live messages arrived since chat paused: desktop's "N new since
 * paused" and the phones' resume button.
 *
 * Anchored to each slice's MONOTONIC live-message counter at the instant the
 * pause begins, which stays accurate under buffer trimming (a messages.length
 * diff does not). Takes every slice key it should total, joined with `|`: a
 * merged feed's count is the sum across its sources. A joined string keeps the
 * identity stable without the caller memoizing. Its per-flush subscription
 * re-renders only the component that calls it, so callers keep it in a small
 * leaf, never the whole chat; while not paused it reads 0 and never re-renders.
 */
export function usePausedNewCount(sliceKeys: string, isPaused: boolean): number {
  const counting = isPaused && !!sliceKeys;
  // The total is the subscription's own value (slices are mutated in place and
  // each flush bumps the store), so nothing reads the store outside it.
  const live = useChatConnectionStore((state) => {
    if (!counting) return 0;
    let n = 0;
    for (const k of sliceKeys.split('|')) n += state.channels.get(k)?.liveMessageCount ?? 0;
    return n;
  });
  // Anchored once per pause, on the render that first sees it (React's
  // adjust-state-while-rendering pattern: no effect, no render-time ref write).
  const [anchor, setAnchor] = useState<number | null>(null);
  if (counting && anchor === null) setAnchor(live);
  if (!counting && anchor !== null) setAnchor(null);
  return counting && anchor !== null ? Math.max(0, live - anchor) : 0;
}
