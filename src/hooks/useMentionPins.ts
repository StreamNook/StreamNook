// Pinned mentions for one chat surface (SN-284). Rust decides which mentions
// are pinned and until when (services/mention_inbox.rs `pins_from`); this
// asks for the chat's rooms and asks again when anything could have changed:
// the inbox changed, the pin minutes changed, the window came back, the
// account changed, or the earliest pin's time is up.
//
// Only the answer to the latest request is drawn, so a slow answer for the
// rooms this chat showed a moment ago (or the account signed in a moment ago)
// never lands on what it shows now.
// With Pin mentions off it listens to nothing and asks nothing.

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { MENTION_INBOX_CHANGED_EVENT } from '../stores/mentionInboxStore';
import { Logger } from '../utils/logger';
import type { MentionPin, MentionPins } from '../types';

/** Rust's `mention_inbox::PINS_EVENT`: the pin minutes changed. */
export const MENTION_PINS_CHANGED_EVENT = 'mention-pins-changed';

/** A little past the earliest pin's end, on Rust's clock, so the next answer
 *  no longer holds it; null with nothing pinned. */
export function nextRefreshDelay(pins: readonly MentionPin[], nowMs: number): number | null {
  if (pins.length === 0) return null;
  const earliest = Math.min(...pins.map((p) => p.until_ms));
  return Math.max(0, earliest - nowMs) + 50;
}

/** Whether two answers draw the same: same entries in the same order, same
 *  ends and copy counts. Keeps the old array so nothing re-renders. */
export function samePins(a: readonly MentionPin[], b: readonly MentionPin[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (p, i) =>
        p.entry.id === b[i].entry.id &&
        p.until_ms === b[i].until_ms &&
        p.run === b[i].run &&
        p.entry.removed === b[i].entry.removed,
    )
  );
}

const NO_PINS: MentionPin[] = [];
/** The longest wait between asks while anything is pinned. */
const MAX_WAIT_MS = 60_000;

/** What was drawn, for which rooms and account. */
interface Answer {
  rooms: string;
  user: string | null;
  pins: MentionPin[];
}
const EMPTY: Answer = { rooms: '', user: null, pins: NO_PINS };

/**
 * The pins for a chat showing `rooms` (Rust room keys joined with `|`; empty
 * while the chat shows no live room). Empty while Pin mentions is off.
 */
export function useMentionPins(rooms: string): MentionPin[] {
  const enabled = useAppStore((s) => s.settings.chat_design?.pin_mentions === true);
  const minutes = useAppStore((s) => s.settings.chat_design?.pin_mentions_minutes);
  const userId = useAppStore((s) => s.currentUser?.user_id ?? null);
  const [answer, setAnswer] = useState<Answer>(EMPTY);
  const seq = useRef(0);

  useEffect(() => {
    if (!enabled || !rooms) return;
    const list = rooms.split('|').filter(Boolean);
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = async () => {
      const mine = ++seq.current;
      let res: MentionPins;
      try {
        res = await invoke<MentionPins>('get_mention_pins', { rooms: list });
      } catch (err) {
        Logger.warn('[MentionPins] get_mention_pins failed:', err);
        return;
      }
      if (disposed || mine !== seq.current) return;
      setAnswer((prev) =>
        prev.rooms === rooms && prev.user === userId && samePins(prev.pins, res.pins)
          ? prev
          : { rooms, user: userId, pins: res.pins.length ? res.pins : NO_PINS },
      );
      if (timer) clearTimeout(timer);
      const delay = nextRefreshDelay(res.pins, res.now_ms);
      // At least once a minute while anything is pinned: a timer can run late
      // across sleep with the window never losing focus.
      timer = delay == null ? undefined : setTimeout(() => void ask(), Math.min(delay, MAX_WAIT_MS));
    };
    void ask();

    const unlisteners: Array<() => void> = [];
    for (const name of [MENTION_INBOX_CHANGED_EVENT, MENTION_PINS_CHANGED_EVENT]) {
      void listen(name, () => void ask()).then((off) => {
        if (disposed) off();
        else unlisteners.push(off);
      });
    }
    // A timer can run late across sleep or a hidden window: ask again on return.
    const onVisible = () => {
      if (document.visibilityState === 'visible') void ask();
    };
    const onFocus = () => void ask();
    document.addEventListener('visibilitychange', onVisible);
    window.addEventListener('focus', onFocus);
    return () => {
      disposed = true;
      // Coming back to these rooms (or turning pins back on) starts empty
      // rather than flashing what was pinned the last time.
      setAnswer(EMPTY);
      if (timer) clearTimeout(timer);
      unlisteners.forEach((off) => off());
      document.removeEventListener('visibilitychange', onVisible);
      window.removeEventListener('focus', onFocus);
    };
  }, [enabled, rooms, minutes, userId]);

  // An answer for other rooms (the chat moved on) or with the switch off is
  // never drawn: derived, so a room change clears at once.
  return enabled && rooms && answer.rooms === rooms && answer.user === userId ? answer.pins : NO_PINS;
}
