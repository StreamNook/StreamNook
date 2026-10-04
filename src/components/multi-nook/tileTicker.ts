// One 4 Hz clock for every MultiNook tile's live state, instead of an
// animation frame per tile. The LIVE button only changes about once a second,
// so sampling every frame across a grid of tiles was pure main-thread cost.
// The clock stops when the last tile unsubscribes and skips its ticks while
// the window is hidden.

import { isWindowHidden } from '../../utils/windowVisibility';

const TICK_MS = 250;
const subscribers = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;

/** Run `fn` on the shared tick. Returns the unsubscribe. */
export function onTileTick(fn: () => void): () => void {
  subscribers.add(fn);
  if (!timer) {
    timer = setInterval(() => {
      if (isWindowHidden()) return;
      subscribers.forEach((f) => f());
    }, TICK_MS);
  }
  return () => {
    subscribers.delete(fn);
    if (subscribers.size === 0 && timer) {
      clearInterval(timer);
      timer = null;
    }
  };
}
