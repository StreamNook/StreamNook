// VOD chat replay: a recorded broadcast's chat, dripped in step with the player.
//
// The engine is Rust's (`services/vod_replay.rs`), the same one the iPhone
// drives: it fetches Twitch's recorded chat, parses it into the rows live chat
// uses, keeps the buffer ahead of the playhead and handles seeks. This store
// only reports the playhead (`vod_replay_tick`, every TICK_MS) and keeps the
// rows Rust hands back, so ChatWidget, the clip modal and the phone's chat pane
// render replay through their ordinary message paths.
//
// The reactive surface is small (`messages` + a `version` counter); the clock
// and the session live in module scope so a tick that adds nothing never
// renders.

import { create } from 'zustand';
import { useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getPlayerControls } from '../keybindings/playerControls';
import type { BackendChatMessage } from '../services/twitchChat';
import { EMPTY_ROOM_STATE } from './chatConnectionStore';
import { Logger } from '../utils/logger';

/** What Rust answers a tick with (`ReplayStep`). */
interface ReplayStep {
  /** Drop every replay row before adding `rows` (a seek, or the first rows). */
  reset: boolean;
  rows: BackendChatMessage[];
  /** Chat for this point is still being fetched. */
  loading: boolean;
  error: string | null;
}

const VISIBLE_MAX = 200; // rendered-message cap (headroom for scroll-back)
const TICK_MS = 750; // playhead report interval
// A seek the buffer cannot answer is fetched on the first report after the
// scrubbing stops. Reporting again this soon after one keeps the old settle time.
const SEEK_SETTLE_MS = 400;

/** Which chat a VOD shows: its replay, or the channel's live chat. */
export type ReplayShowing = 'replay' | 'live';

interface ReplayStore {
  /** True while a VOD replay session is active (drives the VOD Chat / Live Chat switch). */
  active: boolean;
  /** Bumped on each `beginVodReplay`. */
  sessionId: number;
  /** Bumped whenever `messages` changes; the list memo keys off this. */
  version: number;
  messages: BackendChatMessage[];
  error: string | null;
  /** The VOD Chat / Live Chat switch. A new session always starts on replay. */
  showing: ReplayShowing;
}

export const useVodReplayStore = create<ReplayStore>(() => ({
  active: false,
  sessionId: 0,
  version: 0,
  messages: [],
  error: null,
  showing: 'replay',
}));

/** The VOD Chat / Live Chat switch. Replay keeps running underneath live, so
 *  switching back catches up to the playhead at once. */
export function setVodReplayShowing(showing: ReplayShowing): void {
  useVodReplayStore.setState({ showing });
  if (showing === 'replay') tick();
}

// --- session state (module-scoped, non-reactive) -----------------------------
interface Session {
  id: string;
  login: string;
  startOffset: number;
  endOffset: number;
}
let session: Session | null = null;
/** Bumped on every begin and stop, so an answer for an older session is dropped. */
let token = 0;
let visible: BackendChatMessage[] = [];
let ticker: ReturnType<typeof setInterval> | null = null;
let settleTimer: ReturnType<typeof setTimeout> | null = null;
let inFlight = false;
// Where to read the playhead. A clip playing in ClipModal's own <video> passes a
// reader for its session; the phone's player registers one for as long as it is
// mounted (`registerReplayClock`); otherwise the desktop player's controls.
let timeSource: (() => number | null) | null = null;
let playerClock: (() => number | null) | null = null;

/** Rust calls run one after another, so a stop can never land after the begin
 *  that followed it. */
let chain: Promise<unknown> = Promise.resolve();
function call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  const next = chain.then(() => invoke<T>(cmd, args));
  chain = next.catch(() => {});
  return next;
}

/** Playhead in PLAYER time (0 = start of whatever is playing), or null if unavailable. */
function playerTime(): number | null {
  const t = timeSource
    ? timeSource()
    : playerClock
      ? playerClock()
      : (getPlayerControls()?.getCurrentTime() ?? null);
  return t == null || Number.isNaN(t) ? null : t;
}

/** The phone's player: where replay reads the playhead while it is mounted.
 *  Returns the unregister, which leaves a newer player's clock alone. */
export function registerReplayClock(reader: () => number | null): () => void {
  playerClock = reader;
  return () => {
    if (playerClock === reader) playerClock = null;
  };
}

function apply(step: ReplayStep): void {
  const error = step.error ?? null;
  if (step.reset) {
    visible = step.rows.length > VISIBLE_MAX ? step.rows.slice(-VISIBLE_MAX) : step.rows.slice();
  } else if (step.rows.length) {
    visible = visible.concat(step.rows);
    if (visible.length > VISIBLE_MAX) visible = visible.slice(-VISIBLE_MAX);
  } else {
    if (useVodReplayStore.getState().error !== error) useVodReplayStore.setState({ error });
    return;
  }
  // A fresh array, so the list memo re-renders.
  const messages = visible;
  useVodReplayStore.setState((s) => ({ messages, error, version: s.version + 1 }));
}

function tick(): void {
  if (!session || inFlight) return;
  const t = playerTime();
  if (t == null) return; // player not mounted yet
  const mine = token;
  inFlight = true;
  call<ReplayStep>('vod_replay_tick', { videoId: session.id, position: t })
    .then((step) => {
      if (mine !== token) return;
      apply(step);
      // A seek outside what is fetched: report again shortly, which fetches
      // once the playhead has stopped jumping.
      if (step.reset && step.loading && !settleTimer) {
        settleTimer = setTimeout(() => {
          settleTimer = null;
          tick();
        }, SEEK_SETTLE_MS);
      }
    })
    .catch((e) => {
      if (mine === token) Logger.warn('[VodReplay] tick failed:', e);
    })
    .finally(() => {
      if (mine === token) inFlight = false;
    });
}

/** Ends the session on this side; the caller tells Rust when it should. */
function resetLocal(): void {
  if (ticker) {
    clearInterval(ticker);
    ticker = null;
  }
  if (settleTimer) {
    clearTimeout(settleTimer);
    settleTimer = null;
  }
  token++;
  session = null;
  visible = [];
  inFlight = false;
  timeSource = null;
}

/** The source-VOD window to replay for a clip. Both the main player and the clip modal
 *  go through this so they cannot drift apart.
 *
 *  There is NO offset correction here, and that is a measured decision, not an omission.
 *  Twitch anchors a clip's chat replay at exactly `videoOffsetSeconds`: on the clip
 *  `ArtisticFreezingPepperoniNononoCat` (summit1g, reported offset 2781) Twitch's own
 *  page opens with `Ban119: LUL`, and that comment's `contentOffsetSeconds` is 2781:
 *  delta exactly 0.
 *
 *  A previous version subtracted 15 s here, on the theory that the chat reaction spike
 *  sitting ~9 s before the reported start meant the offset ran late. That was a
 *  misreading: a clip is normally cut to START on the reaction, so the burst legitimately
 *  begins at frame one. Do not reintroduce a correction without ground truth of that
 *  form (a named chatter at a known position on Twitch's own clip page). */
export function clipReplayWindow(
  vodOffset: number,
  durationSeconds?: number,
): { startOffset: number; endOffset: number } {
  return { startOffset: Math.max(0, vodOffset), endOffset: vodOffset + (durationSeconds ?? 60) };
}

/** What a replay session needs to be recreated. See `snapshotReplaySession`. */
export interface ReplaySession {
  id: string;
  login: string;
  startOffset: number;
  endOffset: number;
}

export interface BeginReplayOptions {
  /** Seconds into the source VOD where a clip begins. Omit for a VOD (0). */
  startOffset?: number;
  /** Seconds into the source VOD where a clip ends; refills stop past it. */
  endOffset?: number;
  /** Read the playhead from somewhere other than the main player: a clip playing in
   *  ClipModal's own <video>. Deliberately NOT captured by snapshotReplaySession: only
   *  the modal supplies one, and only the modal restores, so a restored session is
   *  always a main-player session. Capturing it would resurrect a dead element ref. */
  getTime?: () => number | null;
}

/** Start a replay session. `login` keys the channel's third-party emote set for parsing.
 *  Safe to call over an existing session (Rust's begin replaces it). */
export function beginVodReplay(id: string, login: string, opts?: BeginReplayOptions): void {
  resetLocal();
  const startOffset = Math.max(0, opts?.startOffset ?? 0);
  const endOffset = opts?.endOffset ?? Number.POSITIVE_INFINITY;
  session = { id, login: login.toLowerCase(), startOffset, endOffset };
  timeSource = opts?.getTime ?? null;
  const mine = token;
  useVodReplayStore.setState((s) => ({
    active: true,
    sessionId: s.sessionId + 1,
    version: 0,
    messages: [],
    error: null,
    showing: 'replay',
  }));
  call('vod_replay_begin', {
    videoId: id,
    login: session.login,
    position: playerTime() ?? 0,
    startOffset,
    endOffset: Number.isFinite(endOffset) ? endOffset : null,
  })
    .then(() => {
      if (mine !== token) return;
      ticker = setInterval(tick, TICK_MS);
      tick();
    })
    .catch((e) => {
      if (mine !== token) return;
      Logger.warn('[VodReplay] could not start:', e);
      useVodReplayStore.setState({ error: 'Could not load chat replay' });
    });
}

/** Tear down the active replay session (called on stopStream / VOD swap). */
export function stopVodReplay(): void {
  const had = session !== null;
  resetLocal();
  if (had) void call('vod_replay_stop', {}).catch(() => {});
  useVodReplayStore.setState({ active: false, messages: [], error: null });
}

/** The session currently driving replay, or null when nothing is running.
 *
 *  There is one session at a time, so a surface that takes it over temporarily (the
 *  clip modal opening on top of a VOD the user is already watching) has to be able to
 *  put back what it interrupted. Without this, closing that modal leaves the VOD playing
 *  with dead chat. */
export function snapshotReplaySession(): ReplaySession | null {
  if (!useVodReplayStore.getState().active || !session) return null;
  return { ...session };
}

/** Put back a session captured with `snapshotReplaySession`. Null just stops replay.
 *  Re-seeds from the player's current position, the same way a seek resync does, so the
 *  restored VOD picks up where it now is rather than where it was. */
export function restoreReplaySession(previous: ReplaySession | null): void {
  if (!previous) {
    stopVodReplay();
    return;
  }
  beginVodReplay(previous.id, previous.login, {
    startOffset: previous.startOffset,
    endOffset: previous.endOffset,
  });
}

const NOOP = async (): Promise<void> => {};
const EMPTY_SET: Set<string> = new Set();
const EMPTY_MAP = new Map();

/** Chat-source object shaped exactly like `useTwitchChat()` / the provider chat,
 *  so ChatWidget can select it with no other structural change. Read-only:
 *  connect/send are no-ops (you can't post into a recorded stream). */
export function useVodReplaySnapshot() {
  const messages = useVodReplayStore((s) => s.messages);
  const version = useVodReplayStore((s) => s.version);
  const error = useVodReplayStore((s) => s.error);
  return useMemo(
    () => ({
      messages,
      connectChat: NOOP,
      sendMessage: NOOP,
      isConnected: true,
      error,
      setPaused: () => {},
      deletedMessageIds: EMPTY_SET,
      clearedUserContexts: EMPTY_MAP,
      roomState: { ...EMPTY_ROOM_STATE },
      userBadges: null,
      liveMessageCount: version,
      // Re-render signal for the memoized message list, same contract as
      // ChannelChatSnapshot.renderToken. Each change bumps `version`.
      renderToken: version,
    }),
    [messages, version, error],
  );
}
