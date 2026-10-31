// Whether a paused chat still shows "Swipe left on a message to reply": one flag for every chat
// pane, kept on this device. It goes for good after the first reply swipe, or once it has shown
// on REPLY_HINT_SHOWS separate pauses (someone who replies from the hold menu never needs it).
import { useSyncExternalStore } from 'react';
import { REPLY_HINT_SHOWS } from './replySwipe';

const LEARNED_KEY = 'sn-swipe-reply-learned';
const SHOWN_KEY = 'sn-swipe-reply-hint-shown';

function readShown(): number {
  try {
    return Number(localStorage.getItem(SHOWN_KEY) ?? '0') || 0;
  } catch {
    return 0;
  }
}

function readLearned(): boolean {
  try {
    return localStorage.getItem(LEARNED_KEY) === '1';
  } catch {
    return false;
  }
}

let learned = readLearned();
const listeners = new Set<() => void>();

function setLearned(): void {
  learned = true;
  try {
    localStorage.setItem(LEARNED_KEY, '1');
  } catch {
    // Private storage is off; the hint just shows again next time.
  }
  listeners.forEach((l) => l());
}

/** A reply swipe ran: the hint has done its job. */
export function learnReplyHint(): void {
  if (!learned) setLearned();
}

/** Chat paused where a reply could start: one more showing, or the last one was already had. */
export function replyHintOnPause(): void {
  if (learned) return;
  const shown = readShown();
  if (shown >= REPLY_HINT_SHOWS) {
    setLearned();
    return;
  }
  try {
    localStorage.setItem(SHOWN_KEY, String(shown + 1));
  } catch {
    // Private storage is off; nothing is counted.
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useReplyHintLearned(): boolean {
  return useSyncExternalStore(subscribe, () => learned, () => learned);
}
