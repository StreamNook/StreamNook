// AutoMod held-message queue, per window. The queue itself lives in Rust
// (services/automod_queue.rs): this store mirrors it from the
// `eventsub://automod-hold` / `eventsub://automod-update` events and seeds
// from `get_automod_queue` when a pane first needs a channel.

import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';

export interface HeldMessage {
  message_id: string;
  channel: string;
  broadcaster_id: string;
  user_id: string;
  user_login: string;
  user_name: string;
  text: string;
  reason: 'automod' | 'blocked_term' | string;
  category?: string;
  level?: number;
  terms?: string[];
  held_at_ms: number;
  status: string;
  /** Why it is held, in Rust's words: "Held by AutoMod · Swearing, level 2". */
  label?: string;
}

/** A held message once decided: the row folds to Rust's one line. */
export interface DecidedHeld {
  held: HeldMessage;
  status: string;
  /** "Allowed Gus's held message". */
  line: string | null;
  moderator: string | null;
}

/** A held message as a chat row draws it. */
export interface HeldRowData {
  held: HeldMessage;
  /** The folded line once decided; null while it waits. */
  line: string | null;
  /** The moderator who decided it. */
  by: string | null;
}

/** How many decided rows a channel keeps for its folded lines. */
const DECIDED_CAP = 50;

interface AutomodState {
  /** channel (lowercase login) -> held rows, oldest first */
  held: Record<string, HeldMessage[]>;
  /** Last resolution per message id, for a brief "Allowed by X" flash. */
  resolved: Record<string, { status: string; moderator?: string | null; at: number }>;
  /** channel -> decided rows, oldest first, so an inline row can fold in place. */
  decided: Record<string, DecidedHeld[]>;
  seed: (channel: string) => Promise<void>;
  resolve: (messageId: string, allow: boolean) => Promise<string | null>;
}

let listenersStarted = false;

function ensureListeners(set: (fn: (s: AutomodState) => Partial<AutomodState>) => void) {
  if (listenersStarted) return;
  listenersStarted = true;
  void listen<HeldMessage>('eventsub://automod-hold', (e) => {
    const row = e.payload;
    if (!row?.message_id || !row.channel) return;
    set((s) => {
      const list = s.held[row.channel] ?? [];
      if (list.some((r) => r.message_id === row.message_id)) return {};
      return { held: { ...s.held, [row.channel]: [...list, row].slice(-100) } };
    });
  }).catch((err) => Logger.warn('[AutoMod] listen hold failed:', err));
  void listen<{ channel: string; message_id: string; status: string; moderator?: string | null; line?: string | null }>(
    'eventsub://automod-update',
    (e) => {
      const { channel, message_id, status, moderator, line } = e.payload;
      set((s) => {
        const list = s.held[channel] ?? [];
        const row = list.find((r) => r.message_id === message_id);
        const decided = row
          ? [...(s.decided[channel] ?? []), { held: row, status, line: line ?? null, moderator: moderator ?? null }].slice(-DECIDED_CAP)
          : s.decided[channel];
        return {
          held: { ...s.held, [channel]: list.filter((r) => r.message_id !== message_id) },
          resolved: { ...s.resolved, [message_id]: { status, moderator, at: Date.now() } },
          ...(decided ? { decided: { ...s.decided, [channel]: decided } } : {}),
        };
      });
    },
  ).catch((err) => Logger.warn('[AutoMod] listen update failed:', err));
}

export const useAutomodStore = create<AutomodState>((set) => ({
  held: {},
  resolved: {},
  decided: {},
  seed: async (channel: string) => {
    ensureListeners(set);
    const key = channel.toLowerCase();
    try {
      const rows = await invoke<HeldMessage[]>('get_automod_queue', { channel: key });
      set((s) => ({ held: { ...s.held, [key]: rows } }));
    } catch (err) {
      Logger.debug('[AutoMod] seed failed:', err);
    }
  },
  resolve: async (messageId: string, allow: boolean) => {
    try {
      await invoke('resolve_automod_message', { messageId, allow });
      return null;
    } catch (err) {
      return typeof err === 'string' ? err : 'AutoMod request failed';
    }
  },
}));
