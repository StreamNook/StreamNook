// Song listening, one per window: the capture /song or the player's note
// started here, or the one this window asked the main window for. The strip
// over a chat's composer and the player's note read it through
// useSyncExternalStore; the stream's sound level has its own feed that never
// goes through React, so the bars move by writing their own transforms.
//
// A window with no player (a MultiChat window, a chat popout) asks the main
// window to listen: it sends `song-identify-request`; the main window answers
// with its state and level as it listens and the result at the end; Cancel
// sends `song-identify-cancel`. These are sent to one window (emitTo), so
// every listener here is the window's own (getCurrentWebviewWindow().listen).

import { emitTo } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { getActiveVideo } from './activeVideo';
import { Logger } from './logger';
import { IS_MOBILE } from './platform';
import type { RecognizeResult } from './songId';

export const IDENTIFY_EVENTS = [
  'song-identify-request',
  'song-identify-state',
  'song-identify-level',
  'song-identify-result',
  'song-identify-cancel',
] as const;
const [REQUEST, STATE, LEVEL, RESULT, CANCEL] = IDENTIFY_EVENTS;

/** The window that holds the player and listens for the others. */
const MAIN = 'main';

export interface Listening {
  id: string;
  /** The chat room the answer goes to (a chat-slice key, lowercased); null
   *  when none was named (the strip then shows nowhere). */
  room: string | null;
  phase: 'listening' | 'finding';
  /** This attempt (from 1) of `attempts`. */
  attempt: number;
  attempts: number;
  /** How long each attempt listens. */
  captureMs: number;
  /** When this attempt's listening really started (ms since the epoch), for
   *  the time left; null until it has. */
  startedAt: number | null;
  /** The window at the other end: the one that asked (in the main window)
   *  or the one listening for this window. */
  remote: { window: string; id: string } | null;
  /** The capture runs in this window (levels come from it). */
  local: boolean;
}

type Remote = { window: string; id: string };

/** The listening that runs, until its capture has settled. */
interface Active {
  id: string;
  local: boolean;
  remote: Remote | null;
  abort: AbortController;
  /** A request's listeners and timers, released when it ends. */
  release: (() => void) | null;
}

let active: Active | null = null;
// What the strip and the note show: cleared at a Cancel, while `active` holds
// on until the capture settles (a new listen meanwhile is busy).
let shown: Listening | null = null;
const watchers = new Set<() => void>();
const levelWatchers = new Set<(level: number) => void>();

function show(next: Listening | null): void {
  shown = next;
  for (const watch of [...watchers]) watch();
}

/** This window's listening, or null. */
export const getListening = (): Listening | null => shown;

/** For useSyncExternalStore (its server snapshot is `() => null`). */
export function subscribeListening(cb: () => void): () => void {
  watchers.add(cb);
  return () => {
    watchers.delete(cb);
  };
}

/** The stream's sound level (0 to 1) while this window listens, for the bars.
 *  Never a React store: write it straight to the bars. */
export function subscribeLevel(cb: (level: number) => void): () => void {
  levelWatchers.add(cb);
  return () => {
    levelWatchers.delete(cb);
  };
}

/** Levels reach the bars at most this often, and another window at most
 *  every FORWARD_MS. */
export const LEVEL_MS = 33;
export const FORWARD_MS = 66;
let levelAt = -Infinity;
let forwardAt = -Infinity;

function publishLevel(level: number): void {
  const now = Date.now();
  if (now - levelAt < LEVEL_MS) return;
  levelAt = now;
  for (const watch of [...levelWatchers]) watch(level);
}

let nextId = 0;
const newId = (): string => `${Date.now().toString(36)}-${(nextId++).toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

const foldRoom = (room: string | null | undefined): string | null => (room ? room.toLowerCase() : null);

function windowLabel(): string {
  try {
    return getCurrentWebviewWindow().label;
  } catch {
    return MAIN;
  }
}

function send(target: string, event: string, payload: unknown): void {
  void emitTo(target, event, payload).catch((e: unknown) => Logger.warn(`[SongId] ${event} not sent:`, e));
}

// ---------------------------------------------------------------- a capture here

/** A capture this window runs, as songId.ts drives it. */
export interface ListenHandle {
  id: string;
  signal: AbortSignal;
  /** The capture's progress. */
  update(patch: Partial<Pick<Listening, 'phase' | 'attempt' | 'startedAt'>>): void;
  /** The stream's sound level now. */
  level(level: number): void;
  /** Stop it as canceled (the player changed under it). */
  abort(): void;
  /** The capture has settled: the window may listen again. */
  end(): void;
}

/** Whether a listening runs here (a new one would be busy). */
export const isListening = (): boolean => active !== null;

/**
 * Start a capture in this window. Null when one already runs: the caller
 * answers busy, and says nothing.
 */
export function beginListening(init: {
  room: string | null;
  attempts: number;
  captureMs: number;
  remote: Remote | null;
}): ListenHandle | null {
  if (active) return null;
  const id = init.remote?.id ?? newId();
  const me: Active = { id, local: true, remote: init.remote, abort: new AbortController(), release: null };
  active = me;
  levelAt = -Infinity;
  forwardAt = -Infinity;
  const mine = () => active === me;
  const forward = (s: Listening) => {
    if (!me.remote) return;
    send(me.remote.window, STATE, {
      id: me.remote.id,
      phase: s.phase,
      attempt: s.attempt,
      attempts: s.attempts,
      captureMs: s.captureMs,
      startedAt: s.startedAt,
    });
  };
  const first: Listening = {
    id,
    room: foldRoom(init.room),
    phase: 'listening',
    attempt: 1,
    attempts: init.attempts,
    captureMs: init.captureMs,
    startedAt: null,
    remote: init.remote,
    local: true,
  };
  show(first);
  forward(first);
  return {
    id,
    signal: me.abort.signal,
    update(patch) {
      if (!mine() || !shown || shown.id !== id) return;
      const next = { ...shown, ...patch };
      show(next);
      forward(next);
    },
    level(level) {
      if (!mine() || me.abort.signal.aborted) return;
      publishLevel(level);
      if (!me.remote) return;
      const now = Date.now();
      if (now - forwardAt < FORWARD_MS) return;
      forwardAt = now;
      send(me.remote.window, LEVEL, { id: me.remote.id, level });
    },
    abort() {
      if (mine()) me.abort.abort();
    },
    end() {
      if (!mine()) return;
      active = null;
      if (shown?.id === id) show(null);
    },
  };
}

/**
 * Cancel the listening (the strip's Cancel, the note's Stop listening). Acts
 * only while listening: once the clip is sent the song is filed within a
 * second. The strip goes at once; a capture here settles a moment later.
 */
export function cancelListening(): void {
  const now = active;
  if (!now || !shown || shown.phase !== 'listening') return;
  if (now.local) {
    now.abort.abort();
    show(null);
    return;
  }
  if (now.remote) send(now.remote.window, CANCEL, { id: now.remote.id });
  finishRequest(now);
}

// ---------------------------------------------------------------- asking the main window

/** A window that holds no player and is not the main window listens through
 *  the main window's player. */
export function listensRemotely(): boolean {
  return !IS_MOBILE && getActiveVideo() === null && windowLabel() !== MAIN;
}

interface IdentifyRequest {
  id: string;
  room: string | null;
  window: string;
}
interface IdentifyState {
  id: string;
  phase: Listening['phase'];
  attempt: number;
  attempts: number;
  captureMs: number;
  startedAt: number | null;
}

/** No first state in this long: the main window holds no player (or is not
 *  there), and the request is called off. */
export const FIRST_STATE_MS = 3000;
/** Each attempt's slack on top of its listening, for the deadline. */
export const ATTEMPT_SLACK_MS = 10_000;

function finishRequest(req: Active, result?: RecognizeResult, room?: string | null): void {
  req.release?.();
  req.release = null;
  if (active === req) {
    active = null;
    if (shown?.id === req.id) show(null);
  }
  if (!result) return;
  void import('./songId').then(({ announceSong }) => announceSong(result, room ?? null));
}

/**
 * Ask the main window to listen for this window (a /song typed where no player
 * is). Its state and level show here as it listens, and the answer is posted
 * in `room`. Says nothing while a listening already runs here.
 */
export async function requestRemoteIdentify(room: string | null): Promise<void> {
  if (active) return;
  const win = getCurrentWebviewWindow();
  const id = newId();
  const remote: Remote = { window: MAIN, id };
  const req: Active = { id, local: false, remote, abort: new AbortController(), release: null };
  active = req;
  levelAt = -Infinity;
  show({
    id,
    room: foldRoom(room),
    phase: 'listening',
    attempt: 1,
    attempts: 1,
    captureMs: 0,
    startedAt: null,
    remote,
    local: false,
  });

  const asked = Date.now();
  const timers: ReturnType<typeof setTimeout>[] = [];
  const stops: Promise<() => void>[] = [];
  let heard = false;
  req.release = () => {
    for (const t of timers) clearTimeout(t);
    for (const stop of stops) void stop.then((f) => f()).catch(() => {});
  };
  const live = () => active === req;

  stops.push(
    win.listen<IdentifyState>(STATE, ({ payload }) => {
      if (payload.id !== id || !live() || !shown || shown.id !== id) return;
      if (!heard) {
        heard = true;
        // The main window listens: it answers within its attempts, or this
        // window lets the listening go quietly (it reloaded, or lost it).
        const deadline = asked + Math.max(1, payload.attempts) * (payload.captureMs + ATTEMPT_SLACK_MS);
        timers.push(setTimeout(() => live() && finishRequest(req), Math.max(0, deadline - Date.now())));
      }
      show({
        ...shown,
        phase: payload.phase,
        attempt: payload.attempt,
        attempts: payload.attempts,
        captureMs: payload.captureMs,
        startedAt: payload.startedAt,
      });
    }),
    win.listen<{ id: string; level: number }>(LEVEL, ({ payload }) => {
      if (payload.id === id && live()) publishLevel(payload.level);
    }),
    win.listen<{ id: string; result: RecognizeResult }>(RESULT, ({ payload }) => {
      if (payload.id === id && live()) finishRequest(req, payload.result, room);
    }),
  );
  // Hear the answer before asking.
  await Promise.all(stops).catch((e: unknown) => Logger.warn('[SongId] could not listen for the main window:', e));
  if (!live()) return;
  timers.push(
    setTimeout(() => {
      if (!live() || heard) return;
      send(MAIN, CANCEL, { id });
      finishRequest(req, { status: 'error', message: 'No stream is playing.' }, room);
    }, FIRST_STATE_MS),
  );
  send(MAIN, REQUEST, { id, room, window: windowLabel() } satisfies IdentifyRequest);
}

// ---------------------------------------------------------------- the main window

let hostInstalled = false;
// Requests called off before their listening began (a late cancel), so the
// main window never listens for nobody.
const calledOff = new Set<string>();
const CALLED_OFF_KEPT = 32;

/**
 * The main window's listener for the other windows' requests, installed once
 * at boot (main.tsx) so a request is heard from the first moment, whatever
 * the player is doing. A no-op in any other window, and the second time.
 */
export function installSongIdentifyHost(): void {
  if (hostInstalled) return;
  hostInstalled = true;
  let win: ReturnType<typeof getCurrentWebviewWindow>;
  try {
    win = getCurrentWebviewWindow();
  } catch {
    return;
  }
  if (win.label !== MAIN) return;
  void win.listen<IdentifyRequest>(REQUEST, ({ payload }) => void serveRequest(payload));
  void win.listen<{ id: string }>(CANCEL, ({ payload }) => {
    if (active?.local && active.remote?.id === payload.id) {
      cancelListening();
      return;
    }
    calledOff.add(payload.id);
    if (calledOff.size > CALLED_OFF_KEPT) calledOff.delete(calledOff.values().next().value as string);
  });
}

async function serveRequest(req: IdentifyRequest): Promise<void> {
  if (!req?.id || !req.window) return;
  const answer = (result: RecognizeResult) => send(req.window, RESULT, { id: req.id, result });
  if (active) {
    answer({ status: 'busy' });
    return;
  }
  const { recognizeNowPlaying } = await import('./songId');
  if (calledOff.delete(req.id)) return;
  // No room here: the answer is posted in the asking window's pane.
  const result = await recognizeNowPlaying(null, { room: null, remote: { window: req.window, id: req.id } });
  answer(result);
}
