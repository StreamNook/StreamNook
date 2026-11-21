// "Show me this mention in the Mentions inbox" (SN-284). A pinned mention from
// another chat opens the inbox of this platform scrolled to that entry and
// highlights it; the row there keeps its own opener (which covers every
// platform and the server's rows). The inbox surfaces (the island's history,
// MultiChat's Mentions mode, the phones' Mentions sheet) listen here: they open
// themselves, find the entry (loading older pages while it is not among the
// loaded ones, a few times) and mark it.

import { useSyncExternalStore } from 'react';

export interface MentionFocus {
  /** The inbox entry's id (any copy of a folded message). */
  id: string;
  /** Rust room key of the entry's chat. */
  room: string;
  /** Orders requests: the same entry asked twice highlights twice. */
  nonce: number;
}

let current: MentionFocus | null = null;
let nonce = 0;
const listeners = new Set<() => void>();

/** Asks the inbox to open at `entry` (the pin's `entry` from Rust). */
export function requestMentionFocus(entry: { id: string; room: string }): void {
  nonce += 1;
  current = { id: entry.id, room: entry.room, nonce };
  listeners.forEach((fn) => fn());
}

/** The inbox found it (or gave up): the request is spent. */
export function settleMentionFocus(n: number): void {
  if (current && current.nonce === n) {
    current = null;
    listeners.forEach((fn) => fn());
  }
}

const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
};
const read = () => current;

/** The waiting request, or null. */
export function useMentionFocus(): MentionFocus | null {
  return useSyncExternalStore(subscribe, read, read);
}

/** How many older pages an inbox loads looking for the entry before giving up. */
export const FOCUS_MAX_PAGES = 5;
/** How long the found row stays highlighted. */
export const FOCUS_HIGHLIGHT_MS = 2400;

/** Test seam. */
export function resetMentionFocus(): void {
  current = null;
  nonce = 0;
  listeners.forEach((fn) => fn());
}
