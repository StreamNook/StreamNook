// The chat's emote of the moment, the emote button's face. Rust counts it
// (`services/emote_moment.rs`) from every row any window receives and sends a
// change as `emote-moment`; this hook asks once for its chat and then follows
// the events for that chat's key. The key is Rust's, returned by the ask: the
// page never builds one (two key spaces must never meet). Every ordering rule
// lives in `createMomentHolder`, which is tested on its own.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ProviderId } from '../types/providers';

export const EMOTE_MOMENT_EVENT = 'emote-moment';

export interface MomentEmote {
  name: string;
  url: string;
  id?: string;
}

/** Rust's answer for one room. `seq` grows with every change of any room. */
export interface MomentAnswer {
  key: string;
  seq: number;
  emote: MomentEmote | null;
}

/**
 * One chat's moment as answers arrive in any order. Events heard before the
 * ask answers are kept (newest per key) and reconciled with it by `seq`;
 * events for other chats, and anything older than what is held, change
 * nothing. Each method says whether the moment shown changed.
 */
export function createMomentHolder() {
  let key: string | null = null;
  let held: MomentAnswer | null = null;
  const early = new Map<string, MomentAnswer>();
  const take = (next: MomentAnswer): boolean => {
    if (next.key !== key || (held && next.seq < held.seq)) return false;
    const changed = held?.emote?.url !== next.emote?.url || held?.emote?.name !== next.emote?.name;
    held = next;
    return changed;
  };
  return {
    onEvent(next: MomentAnswer): boolean {
      if (key === null) {
        const prev = early.get(next.key);
        if (!prev || next.seq >= prev.seq) early.set(next.key, next);
        return false;
      }
      return take(next);
    },
    onAnswer(answer: MomentAnswer): boolean {
      key = answer.key;
      const seen = early.get(answer.key);
      early.clear();
      const a = take(answer);
      const b = seen ? take(seen) : false;
      return a || b;
    },
    current(): MomentEmote | null {
      return held?.emote ?? null;
    },
  };
}

/**
 * The emote of the moment of the chat named by `provider` and `channel` (as
 * the composer names it to its emote matching), or null in a quiet chat.
 * Mounted inside the button, so a change re-renders only the button.
 */
export function useEmoteMoment(provider: ProviderId | null, channel: string | null | undefined): MomentEmote | null {
  const asked = provider && channel ? `${provider}\n${channel}` : '';
  const [shown, setShown] = useState<{ asked: string; emote: MomentEmote | null }>({ asked: '', emote: null });
  useEffect(() => {
    if (!provider || !channel) return;
    let alive = true;
    let unlisten: (() => void) | undefined;
    const holder = createMomentHolder();
    const show = () => setShown({ asked: `${provider}\n${channel}`, emote: holder.current() });
    // Listen first, so no change between the ask and the listener is lost.
    void listen<MomentAnswer>(EMOTE_MOMENT_EVENT, (e) => {
      if (alive && holder.onEvent(e.payload)) show();
    })
      .then((u) => {
        if (!alive) {
          u();
          return null;
        }
        unlisten = u;
        return invoke<MomentAnswer>('get_emote_moment', { provider, channel });
      })
      .then((answer) => {
        if (alive && answer && holder.onAnswer(answer)) show();
      })
      .catch(() => {
        // No moment is the smiley; nothing to tell anyone.
      });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [provider, channel]);
  // Another chat's moment never shows while this one's answer is on its way.
  return shown.asked === asked ? shown.emote : null;
}
