// Raw facts the main window reports to Discord presence, and the serial sender
// that delivers them.
//
// Why TS: Rust decides everything the Discord status says
// (src-tauri/src/services/discord_presence.rs). Two of its inputs exist only in
// the main window's stores: the MultiNook grid (multiNookStore owns the slots;
// Rust's settings copy lags an in-flight save) and the player (AppStore's
// currentMediaType, streamUrl, vodPlayback). These functions copy those fields
// out as they are; they rank, filter and decide nothing.

import type { MultiNookSlot, TwitchStream } from '../types';

/** One MultiNook tile, as Rust's `SlotFacts`. */
export interface SlotFacts {
  provider: string;
  login: string;
  user_id: string;
  name: string;
  title: string;
  category: string;
  avatar_url: string;
  maximized: boolean;
  minimized: boolean;
  chat_focused: boolean;
  offline: boolean;
  load_error: boolean;
}

/** The main player, as Rust's `MediaFacts`. */
export interface MediaFacts {
  media_type: string;
  playing: boolean;
  vod: boolean;
  url: string;
  login: string;
  user_id: string;
  name: string;
  title: string;
  category: string;
  avatar_url: string;
}

export type WriteResult = 'applied' | 'ignored' | 'stale';

export interface GridInput {
  isMultiNookActive: boolean;
  maximizedSlotId: string | null;
  slots: MultiNookSlot[];
}

/** The grid's tiles, or null when the grid is closed or empty.
 *  `chatFocusedId` is the id of the slot whose chat the grid shows. */
export function gridFacts(input: GridInput, chatFocusedId: string | null): SlotFacts[] | null {
  if (!input.isMultiNookActive || input.slots.length === 0) return null;
  return input.slots.map((s) => ({
    provider: s.provider ?? 'twitch',
    login: s.channelLogin,
    user_id: s.channelId ?? '',
    name: s.channelName ?? '',
    title: s.title ?? '',
    category: s.gameName ?? '',
    avatar_url: s.profileImageUrl ?? '',
    maximized: s.id === input.maximizedSlotId,
    minimized: !!s.isMinimized,
    chat_focused: s.id === chatFocusedId,
    offline: s.offline === true,
    load_error: !!s.loadError,
  }));
}

export interface MediaInput {
  currentMediaType: string | null;
  streamUrl: string | null;
  vodPlayback: unknown;
  originalMediaUrl: string | null;
  currentStream: Pick<
    TwitchStream,
    'user_login' | 'user_id' | 'user_name' | 'title' | 'game_name' | 'profile_image_url'
  > | null;
}

/** The player's facts while it shows a clip, a VOD or an offline room, else null. */
export function mediaFacts(m: MediaInput): MediaFacts | null {
  const type = m.currentMediaType;
  if (type !== 'clip' && type !== 'video' && type !== 'offline_chat') return null;
  const s = m.currentStream;
  return {
    media_type: type,
    playing: !!m.streamUrl && m.streamUrl !== 'offline',
    vod: m.vodPlayback != null,
    url: m.originalMediaUrl ?? '',
    login: s?.user_login ?? '',
    user_id: s?.user_id ?? '',
    name: s?.user_name ?? '',
    title: s?.title ?? '',
    category: s?.game_name ?? '',
    avatar_url: s?.profile_image_url ?? '',
  };
}

export interface SenderDeps {
  /** Sends one value: `(gen, seq, value)` to the command. */
  send: (gen: number, seq: number, value: unknown) => Promise<WriteResult>;
  /** The page generation now held. */
  gen: () => number;
  /** Reads the page generation again from Rust. */
  refreshGen: () => Promise<void>;
  /** Schedules a retry; returns a cancel. Injected for tests. */
  later: (fn: () => void, ms: number) => () => void;
  /** The clock, in ms (`Date.now`). Injected for tests. */
  now: () => number;
}

export interface Sender<T> {
  /** Report a value; equal to the newest known one, it sends nothing. */
  push(value: T | null): void;
  /** Forget what was delivered, so the next push sends even an equal value. */
  reset(): void;
}

/** One value at a time to one command: a newer value replaces a queued one,
 *  `stale` re-reads the generation and resends, `ignored` is final (another
 *  window, or an older number), a failure retries after 5 s or on the next
 *  change, and "delivered" moves only on `applied`. */
export function createSender<T>(deps: SenderDeps): Sender<T> {
  let delivered: string | null = null;
  let flying: string | null = null;
  let queued: { json: string; value: T | null } | null = null;
  let seq = 0;
  let cancelRetry: (() => void) | null = null;
  // Bumped by `reset`: a write in flight from before it no longer counts as
  // delivered (Rust may have cleared it just after applying it).
  let epoch = 0;
  let flyingEpoch = 0;

  const newest = () => queued?.json ?? (flyingEpoch === epoch ? flying : null) ?? delivered;

  const run = async (): Promise<void> => {
    if (flying !== null || cancelRetry || !queued) return;
    const job = queued;
    queued = null;
    flying = job.json;
    flyingEpoch = epoch;
    let retry = false;
    try {
      // Increasing, and never below the clock: a sender rebuilt in the same
      // page (a hot reload) still counts above the one before it.
      seq = Math.max(seq + 1, deps.now());
      const result = await deps.send(deps.gen(), seq, job.value);
      const current = flyingEpoch === epoch;
      if (result === 'applied') {
        if (current) delivered = job.json;
        else queued ??= job;
      } else if (result === 'stale') {
        await deps.refreshGen();
        queued ??= job;
      } else if (!current) {
        queued ??= job;
      }
    } catch {
      queued ??= job;
      retry = true;
    } finally {
      flying = null;
    }
    if (retry) {
      cancelRetry = deps.later(() => {
        cancelRetry = null;
        void run();
      }, 5000);
      return;
    }
    if (queued) void run();
  };

  return {
    push(value) {
      const json = JSON.stringify(value);
      if (json === newest()) return;
      queued = { json, value };
      if (cancelRetry) {
        // A change retries at once.
        cancelRetry();
        cancelRetry = null;
      }
      void run();
    },
    reset() {
      delivered = null;
      epoch += 1;
    },
  };
}
