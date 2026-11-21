// What the app itself puts over (or takes away from) the chat column, for the
// pinned mentions' "is the stack in front of the viewer" rule (SN-284,
// `chatSurfaceShown`). Each owner of such a thing sets its key while it is up:
// the drops drawer, the emote picker, a collapsed hover-reveal column. A key
// is app-wide on purpose: when one of them is up, no chat surface counts it as
// in front.

import { useSyncExternalStore } from 'react';

const covers = new Set<string>();
const listeners = new Set<() => void>();

export function setChatCover(key: string, on: boolean): void {
  if (on === covers.has(key)) return;
  if (on) covers.add(key);
  else covers.delete(key);
  listeners.forEach((fn) => fn());
}

export function isChatCovered(): boolean {
  return covers.size > 0;
}

const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
};

/** Whether any cover is up. */
export function useChatCovered(): boolean {
  return useSyncExternalStore(subscribe, isChatCovered, isChatCovered);
}

/** Test seam. */
export function resetChatCovers(): void {
  covers.clear();
  listeners.forEach((fn) => fn());
}
