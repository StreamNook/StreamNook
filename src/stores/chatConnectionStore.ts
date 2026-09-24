// Per-channel Twitch chat state, owned by a single shared WebSocket bridge to
// the Rust IRC service. Multiple consumers (the main app's ChatWidget plus N
// MultiChat tab widgets) acquire channels through this store; the store
// reference-counts subscribers so we hold exactly one IRC connection regardless
// of how many UI surfaces are viewing the same channel.
//
// Wire-format contract with the Rust backend (`src-tauri/src/services/irc_service.rs`):
//   • Native IRC frames (PRIVMSG / USERNOTICE / etc.) carry `#channel` in their
//     own text and are routed here by inspecting that substring.
//   • Synthetic events `USER_BADGES:#<channel>:<badges>`,
//     `{"type":"ROOMSTATE","channel":…}`, `{"type":"CLEARMSG","channel":…}`,
//     `{"type":"CLEARCHAT","channel":…}` carry the channel explicitly.
//   • Global events `HEARTBEAT`, `IRC_CONNECTED`, `RECONNECTING:n`,
//     `RECONNECTED`, `RECONNECT_*`, `CONNECTION_WARNING:…` are not channel-
//     scoped and apply to every active channel slice.
//
// Channel keys are stored lowercase — IRC frames always carry lowercase, so
// upstream callers using mixed case still resolve correctly via `.toLowerCase()`
// at the API boundary.

import { sameSentContent } from '../utils/sentContent';
import { isWindowHidden, onWindowVisibility } from '../utils/windowVisibility';
import { IS_MOBILE } from '../utils/platform';
import { useEffect, useState } from 'react';
import {
  CHAT_BUFFER_SIZE,
  currentBufferLimit,
  liveAppendLimit,
  resumeOverflowFor,
  trimWithEventRetention,
} from './chatBufferTrim';
import { create } from 'zustand';
import { useShallow } from 'zustand/react/shallow';
import { PROVIDERS, type ProviderId } from '../types/providers';
import { makeKey, parseKey, sliceLookupKey } from '../utils/providerKey';
import { streamProvider } from '../utils/streamProvider';
import { parseBadges } from '../services/twitchBadges';
import { invoke } from '@tauri-apps/api/core';
import { fetchAllEmotes, fetchKickChannelEmotes, fetchYouTubeChannelEmotes, enhanceRustEmotes, type Emote, type EmoteSet } from '../services/emoteService';
import { Logger } from '../utils/logger';
import { useAppStore } from './AppStore';
import { useGiftBombStore, type GiftRecipient } from './giftBombStore';
import { giftBombOriginOf, isGiftBombAnnouncement, isGiftBombChild } from '../utils/giftBombCollapse';
import { useMessageRepeatStore, type RepeatParticipant } from './messageRepeatStore';
import { normalizeForRepeat, isPrivilegedChatter } from '../utils/messageRepeat';
import { tokenizeLocalBody } from '../utils/localMessageTokens';
import type { SongMatch } from '../utils/songId';
import type { BackendChatMessage } from '../services/twitchChat';

// Hard caps borrowed from the prior single-channel hook. Keeping them as
// per-channel limits means a 5-channel MultiChat caps memory at 5x the
// historical single-channel ceiling — bounded and predictable.
const CHAT_HISTORY_MAX = 100;
const CHAT_MAX_WITH_BUFFER = CHAT_HISTORY_MAX + CHAT_BUFFER_SIZE;

// Reconnection is UNBOUNDED by design: a capped ladder ended in a permanent
// dead state for anyone with a flaky connection. This only controls wording —
// early attempts name the delay, later ones just say we are still trying.
const RECONNECT_QUIET_ATTEMPTS = 3;
const WS_OPEN_RETRY_ATTEMPTS = 5;
const HEALTH_CHECK_INTERVAL_MS = 30_000;
const STALE_WARNING_MS = 2 * 60_000;
const STALE_RECONNECT_MS = 3 * 60_000;

// --- Public types -----------------------------------------------------------

export interface ModerationContext {
  type: 'timeout' | 'ban' | 'deleted';
  duration?: number;
  username?: string;
}

export interface ClearedUserEntry {
  context: ModerationContext;
  affectedMessageIds: Set<string>;
}

export interface RoomState {
  followersOnly: number; // -1 off, 0 any followers, >0 minutes
  slow: number;
  subsOnly: boolean;
  emoteOnly: boolean;
  r9k: boolean;
}

export const EMPTY_ROOM_STATE: RoomState = {
  followersOnly: -1,
  slow: 0,
  subsOnly: false,
  emoteOnly: false,
  r9k: false,
};

export interface SendUserInfo {
  username: string;
  displayName: string;
  userId: string;
  color?: string;
  badges?: string;
}

/** Identity of a chosen secondary account to send a message AS. */
export interface SendAsAccount {
  userId: string;
  login: string;
  displayName: string;
  color?: string;
}

interface ChannelSlice {
  channel: string;
  /** Source platform. Twitch keeps bare-login keys; non-Twitch sources are
   *  keyed "provider:channel". MultiChat only; the main app is always twitch. */
  provider: ProviderId;
  channelId: string | null;
  messages: any[];
  isConnected: boolean;
  error: string | null;
  roomState: RoomState;
  userBadges: string | null;
  deletedMessageIds: Set<string>;
  clearedUserContexts: Map<string, ClearedUserEntry>;
  /** Currently pinned message (provider-driven; e.g. Kick's pin event). */
  pinnedMessage: any | null;
  refCount: number;
  isPausedForBuffer: boolean;
  /** Rows above the cap still allowed after a resume; set by setChannelPaused,
   *  released RESUME_DECAY_PER_FLUSH per flush by flushPending. */
  resumeOverflow: number;
  /** Monotonic count of live messages appended to this channel since the slice
   *  was created. NEVER decremented — buffer trimming, moderation removals, and
   *  the cap don't touch it. This is the reliable baseline for "N new messages
   *  since you paused": `messages.length` can't be used because it's capped and
   *  trimmed. Historical backfill (prepended, not live) is intentionally
   *  excluded — only `pushMessage` bumps it. */
  liveMessageCount: number;
  // Internals (not surfaced via the per-channel hook):
  seenMessageIds: Set<string>;
  /** Real Helix ids stamped onto our own optimistic rows, still awaiting their
   *  IRC echo. Gates the per-message own-echo upgrade scan: only these ids can
   *  ever match it, so every other incoming message skips the O(buffer)
   *  findIndex it used to pay. Consumed on echo (hit or miss - a miss means
   *  the content-match reconciliation already replaced the row). */
  pendingUpgradeIds: Set<string>;
  /** IRC USERSTATE badges string, used to repaint optimistic messages with the
   *  caller's tenure-correct badges for the channel. */
  userBadgesFromIrc: string | null;
  /** The connected user's own chat color from USERSTATE. Lets own optimistic
   *  messages paint in the real color from the first frame instead of flashing
   *  a default until the IRC echo round-trips. */
  userColorFromIrc: string | null;
  /** Join hold: live rows that arrived before the Rust backfill landed. They
   *  paint together with the history in one revision, so a freshly joined
   *  pane never shows a live tail first and a prepended block a second later.
   *  Released by the backfill, by its own cap (HISTORY_HOLD_MS), by an own
   *  send or system row, and by any immediate flush a moderation event asks
   *  for. */
  historyHold: { held: any[]; timer: ReturnType<typeof setTimeout> } | null;
}

interface ChatConnectionState {
  channels: Map<string, ChannelSlice>;
  wsPort: number | null;
  /** Bumped any time something inside a channel slice mutates in place. Used
   *  by the per-channel hook to drive re-renders without forcing the store to
   *  fully re-create slice objects (the slice holds Sets/Maps that we mutate
   *  in place for perf, which Zustand wouldn't otherwise notice). */
  revision: number;
  /** Per-channel change counters ALONGSIDE the global revision. Channel-scoped
   *  consumers (useChannelChat and the tab counters) subscribe to their own
   *  key so a flood in one channel no longer re-renders every mounted pane;
   *  cross-channel consumers (BlendedChatPane sources, LiveOverlayFeed,
   *  useChannelSocial) keep the global signal. */
  revisionByChannel: Record<string, number>;
}

export const useChatConnectionStore = create<ChatConnectionState>(() => ({
  channels: new Map(),
  wsPort: null,
  revision: 0,
  revisionByChannel: {},
}));

// --- Module-scope mutable bridge state --------------------------------------
//
// The WebSocket and its associated timers live outside the Zustand state
// because (a) they are not React-reactive values, and (b) keeping them in
// closures avoids subtle issues with stale references inside the WS callbacks.

let ws: WebSocket | null = null;
// The in-flight bridge connect, or null. This is the ONE source of truth for
// "a connect is running" (a separate boolean could disagree with it, and a
// concurrent caller that reads a stale flag is exactly how chat used to strand:
// the second caller returned as if it had connected, so nothing retried and the
// socket stayed null forever). Concurrent callers await this promise and then
// verify the socket really opened.
let wsConnectPromise: Promise<void> | null = null;

// Bumped whenever the last channel is released. A connect captures this on entry
// and discards its socket if the value moved, because a connect can easily
// outlive the thing that asked for it (start_chat plus the WS-open retries can
// run for tens of seconds, and a pane can be closed in that window).
let connectGeneration = 0;

/** Whether a live socket is currently open. Read through a function so callers
 *  that assigned `ws = null` earlier in their own flow still see the value as
 *  it is NOW (an awaited connect reassigns it, which control-flow narrowing in
 *  the caller cannot know about). */
function socketIsOpen(): boolean {
  return !!ws && ws.readyState === WebSocket.OPEN;
}

/** Reject if `p` has not settled in `ms`. The bridge connect must be bounded:
 *  `wsConnectPromise` and `reconnectInFlight` are what the watchdog reads to
 *  decide a recovery is already under way, so a connect that never settles
 *  (the Rust side takes a process-global start lock) would disable the
 *  watchdog for the rest of the session. */
function withTimeout<T>(p: Promise<T>, ms: number, label: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
    p.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });
}

// Generous: start_chat can legitimately spend ~15s connecting IRC plus a
// handshake, and it queues behind a process-global lock. This is a deadman for
// a wedged backend, not a latency budget.
const BRIDGE_CONNECT_TIMEOUT_MS = 45_000;
let reconnectAttempts = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let healthCheckTimer: ReturnType<typeof setInterval> | null = null;
let lastMessageTime = Date.now();
let intentionalDisconnect = false;
let currentUserId: string | null = null;
// True between a backend IRC_RECONNECTING and the next IRC_CONNECTED; gates
// the missed-message backfill so a first connect doesn't double-preload.
let backendReconnecting = false;
// Approximate start of the current outage: the last time any frame arrived
// when IRC_RECONNECTING was first seen. Bounds the backfill fetch window.
let outageStartedAtMs: number | null = null;
// Consecutive watchdog reconnects without an IRC_CONNECTED in between. At 2,
// the backend task is alive but wedged (start_chat's idempotent path can't
// fix that), so the watchdog escalates to a full stop_chat teardown.
let watchdogCycles = 0;
// Stale-ladder stage 1 marker: set when the watchdog has asked the backend to
// nudge (re-JOIN) its channels for the current quiet spell; any arriving frame
// clears it. Non-null when the next full stale window should escalate to the
// reconnect path instead of nudging again.
let staleNudgeAtMs: number | null = null;
// Serializes reconnectAll: the retry timer and a user-triggered refresh must
// not tear the socket down concurrently.
let reconnectInFlight = false;
// A reconnect requested while another was in flight. Without this the request
// is discarded and nothing ever retries.
let reconnectPending = false;
// Same, for callers whose goal is a REBUILT socket rather than merely a live
// one (a hard channel refresh, or re-authing after an account switch). For them
// "a socket is already open" is not success, so they must survive the
// socketIsOpen() shortcut below.
let reconnectForcePending = false;
// Armed when a lost-session IRC_RECONNECTING arrives; fires the visible
// "connection lost" row + pane error only if the outage outlives the grace
// window. Fast silent rebuilds (the common case) show nothing, and the gap
// backfill covers the missed messages either way, because outageStartedAtMs
// is stamped at outage START regardless of whether the row ever fires.
let pendingLostRowTimer: ReturnType<typeof setTimeout> | null = null;
// Mirrors the backend's SESSION_FLAP_THRESHOLD_MS: a rebuild the backend
// considers healthy (0-1s reconnect delay + a few seconds of handshake) fits
// comfortably inside the window.
const LOST_ROW_GRACE_MS = 10_000;

function clearPendingLostRow(): void {
  if (pendingLostRowTimer !== null) {
    clearTimeout(pendingLostRowTimer);
    pendingLostRowTimer = null;
  }
}

// Every Twitch user id that belongs to the local user (primary + any linked
// secondary accounts). Used so a message we sent from a secondary account is
// recognized as "own" during optimistic reconciliation, even though the IRC
// reader's identity is always the primary. Populated by the send-account store.
let ownAccountIds = new Set<string>();

export function setOwnAccountIds(ids: string[]): void {
  ownAccountIds = new Set(ids);
}

function isOwnUserId(userId: string | null | undefined): boolean {
  if (!userId) return false;
  return userId === currentUserId || ownAccountIds.has(userId);
}

// The last own chat color USERSTATE reported, persisted so a cold launch can
// seed new channel slices and paint the first optimistic message correctly
// before this session's USERSTATE has arrived. USERSTATE refreshes it on every
// JOIN and after every send, so a color change propagates on its own.
const OWN_COLOR_KEY = 'streamnook:lastOwnChatColor';

function lastOwnChatColor(): string | null {
  try {
    return localStorage.getItem(OWN_COLOR_KEY);
  } catch {
    return null;
  }
}

function persistOwnChatColor(color: string): void {
  try {
    localStorage.setItem(OWN_COLOR_KEY, color);
  } catch {
    // Storage unavailable (private mode / quota); the in-memory slice cache
    // still removes the flash for this session.
  }
}

// Shared per-channel emote cache, so split panes on the same channel hold one
// EmoteSet between them rather than a copy each.
//
// Keyed strictly by lowercase channel login, so 7TV emotes sharing a name
// across channels never collide: same name, different ids and URLs.
const emoteCache = new Map<string, EmoteSet>();
const inflightEmoteFetches = new Map<string, Promise<EmoteSet | null>>();
const emoteSubscribers = new Map<string, Set<() => void>>();

// Chat-side gift-bomb collapse: a submysterygift announces N gifts, and its N
// subgift follow-ups share an origin id. With collapse on, only the
// announcement row is kept; children route to the activity path and their
// recipients feed giftBombStore for the announcement card.
//
// A child collapses only once its announcement has been seen, so a lone gift
// still renders as its own card. Children arriving before the announcement
// render briefly, then fold out via foldBufferedGiftChildren. Origin ids are
// globally unique, so a pruned origin cannot collide with a later bomb; the
// set is bounded only to cap memory. Mirrors OverlayChat.collapseGiftBombs
// through the shared matchers in giftBombCollapse.
const announcedGiftBombOrigins = new Set<string>();
const MAX_TRACKED_BOMB_ORIGINS = 200;

// Open repeat runs per channel: normalized message text -> the run's anchor.
// `pushSeq` is the value of that channel's push counter when the anchor landed,
// so we can tell whether the anchor has since been trimmed out of the buffer
// without scanning it. A run whose anchor is gone must not swallow later copies,
// or they'd vanish with nothing on screen carrying their count.
interface RepeatRun {
  anchorId: string;
  /** Last time a copy joined. Slides, so a sustained wave stays one run. */
  atMs: number;
  pushSeq: number;
  /** Messages in the run, including the anchor. */
  count: number;
  participants: RepeatParticipant[];
}
const openRepeatRuns = new Map<string, Map<string, RepeatRun>>();
// Monotonic count of messages pushed per channel. Only ever incremented and
// compared, so wraparound isn't a practical concern.
const channelPushSeq = new Map<string, number>();
const MAX_OPEN_RUNS_PER_CHANNEL = 200;

function pruneRepeatRuns(runs: Map<string, RepeatRun>, nowMs: number, windowMs: number): void {
  for (const [key, run] of runs) {
    if (nowMs - run.atMs > windowMs) runs.delete(key);
  }
  while (runs.size > MAX_OPEN_RUNS_PER_CHANNEL) {
    const oldest = runs.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    runs.delete(oldest);
  }
}

/** Whether the logged-in user moderates the channel this slice belongs to.
 *  Reads the badges already cached on the slice, so it costs nothing. */
function isModeratorOfSlice(slice: ChannelSlice): boolean {
  const badges = slice.userBadges ?? slice.userBadgesFromIrc ?? '';
  if (!badges) return false;
  return /\bmoderator\/|\bbroadcaster\/|\bglobal_mod\//.test(badges);
}

/** Forget every open run for a channel (channel switch, disconnect, clear). */
export function resetRepeatRuns(channelKey?: string): void {
  if (channelKey) {
    openRepeatRuns.delete(channelKey.toLowerCase());
    channelPushSeq.delete(channelKey.toLowerCase());
    return;
  }
  openRepeatRuns.clear();
  channelPushSeq.clear();
}

// Extract the gift-bomb origin + recipient from a buffered message, if it is a
// (non-suppressed, out-of-order) gift child. Raw-string rows and non-gift rows
// return null. Used to retroactively fold early children into the card.
function bufferedGiftChildInfo(m: any): { origin: string; recipient?: GiftRecipient } | null {
  if (!m || typeof m !== 'object') return null;
  const mt = m.metadata?.msg_type || m.tags?.['msg-id'];
  if (!isGiftBombChild(mt)) return null;
  const origin = giftBombOriginOf(m.tags);
  if (!origin) return null;
  const rid = m.tags?.['msg-param-recipient-id'];
  const recipient: GiftRecipient | undefined = rid
    ? {
        userId: rid,
        userName: m.tags?.['msg-param-recipient-user-name'] || '',
        displayName:
          m.tags?.['msg-param-recipient-display-name'] || m.tags?.['msg-param-recipient-user-name'] || '',
      }
    : undefined;
  return { origin, recipient };
}

// When an announcement arrives after some of its children, pull those children
// back out of both the live buffer and the pending flush queue and fold their
// recipients into the card. Returns how many rows were removed.
function foldBufferedGiftChildren(slice: ChannelSlice, origin: string): number {
  const store = useGiftBombStore.getState();
  let removed = 0;
  const scrub = (arr: any[]) => {
    for (let i = arr.length - 1; i >= 0; i--) {
      const info = bufferedGiftChildInfo(arr[i]);
      if (info && info.origin === origin) {
        if (info.recipient) store.addRecipient(origin, info.recipient);
        arr.splice(i, 1);
        removed++;
      }
    }
  };
  // The live buffer is copied before scrubbing so its identity changes only
  // when rows were actually removed; the pending queue is never rendered.
  const live = slice.messages.slice();
  scrub(live);
  if (live.length !== slice.messages.length) slice.messages = live;
  const pending = pendingByChannel.get(slice.channel);
  if (pending) scrub(pending);
  return removed;
}

function notifyEmoteSubscribers(channelKey: string) {
  const subs = emoteSubscribers.get(channelKey);
  if (!subs) return;
  for (const cb of subs) {
    try {
      cb();
    } catch (err) {
      Logger.warn('[ChatStore] emote subscriber callback threw:', err);
    }
  }
}

/** Subscribe to emote-cache changes for a specific channel. Returns an
 *  unsubscribe function. Used by `useChannelEmotes` to drive re-renders. */
export function subscribeChannelEmotes(channel: string, cb: () => void): () => void {
  const key = channel.toLowerCase();
  let set = emoteSubscribers.get(key);
  if (!set) {
    set = new Set();
    emoteSubscribers.set(key, set);
  }
  set.add(cb);
  return () => {
    const s = emoteSubscribers.get(key);
    if (s) {
      s.delete(cb);
      if (s.size === 0) emoteSubscribers.delete(key);
    }
  };
}

/** Returns the cached EmoteSet for a channel if present, else null. Does NOT
 *  fetch — call `ensureChannelEmotes` first or alongside. */
export function getChannelEmotes(channel: string, provider: ProviderId = 'twitch'): EmoteSet | null {
  return emoteCache.get(emoteCacheKey(channel, provider)) ?? null;
}

/** Fetch the channel's emote set if not already cached. Coalesces concurrent
 *  callers via inflight tracking so 3 ChatWidget instances mounting the same
 *  channel all share one network round-trip. */
/**
 * Force re-fetch emotes for a channel by busting the frontend cache and
 * re-running the fetch pipeline. Used by /refresh. The Rust-side emote cache
 * has its own 5-minute TTL, so very-recent re-fetches may return cached data
 * from the backend; the frontend bust still triggers a fresh re-render of the
 * picker so the user sees the latest state.
 */
export async function refreshChannelEmotes(
  channel: string,
  channelId: string,
  // Without this the provider defaulted to twitch, so a YouTube channel id was
  // sent to Twitch's Helix emote API, which answers 400 ("broadcaster_id must be
  // numeric"). Absent still means twitch, so Twitch callers are unchanged.
  provider: ProviderId = 'twitch',
): Promise<EmoteSet | null> {
  // The cache is keyed by emoteCacheKey (provider-namespaced for Kick and
  // YouTube). Busting the bare login here missed those entries, so a live 7TV
  // change on a Kick or YouTube channel never reached the picker: the stale set
  // was handed straight back (2026-09-07).
  const key = emoteCacheKey(channel, provider);
  emoteCache.delete(key);
  inflightEmoteFetches.delete(key);
  return ensureChannelEmotes(channel, channelId, provider);
}

/**
 * Patch this window's cached set with the composed 7TV delta Rust emitted for a
 * live emote-set change: drop the rows it names (by id AND name, since one emote
 * can legitimately sit under two aliases), then add the rows it sends, which
 * already include any global a removal stopped shadowing. No fetch. A fresh
 * object is stored so identity-keyed indexes (getEmoteLookup) rebuild and
 * subscribers re-render. Returns false when nothing is cached for the channel;
 * the next ensureChannelEmotes fetches the already-patched Rust cache.
 */
export function applyChannelEmoteDelta(
  channel: string,
  provider: ProviderId,
  composed: { added: Emote[]; removed: { id: string; name: string }[] },
): boolean {
  const key = emoteCacheKey(channel, provider);
  const current = emoteCache.get(key);
  if (!current) return false;
  const rowKey = (id: string, name: string) => `${id}\u0000${name}`;
  const gone = new Set(composed.removed.map((r) => rowKey(r.id, r.name)));
  const kept = gone.size
    ? current['7tv'].filter((e) => !gone.has(rowKey(e.id, e.name)))
    : current['7tv'].slice();
  const next: EmoteSet = { ...current, '7tv': [...kept, ...enhanceRustEmotes(composed.added)] };
  emoteCache.set(key, next);
  notifyEmoteSubscribers(key);
  return true;
}

// The emote-cache key namespaces non-Twitch providers so the SAME channel slug on
// two platforms (e.g. xqc on Twitch and Kick) keeps separate emote sets. Twitch
// stays a bare login so its path is byte-identical.
export function emoteCacheKey(channel: string, provider: ProviderId = 'twitch'): string {
  const c = channel.toLowerCase();
  return provider === 'twitch' ? c : `${provider}:${c}`;
}

export async function ensureChannelEmotes(
  channel: string,
  channelId: string,
  provider: ProviderId = 'twitch',
): Promise<EmoteSet | null> {
  const key = emoteCacheKey(channel, provider);
  const cached = emoteCache.get(key);
  if (cached) return cached;
  const inflight = inflightEmoteFetches.get(key);
  if (inflight) return inflight;

  const promise = (async () => {
    try {
      // Kick and YouTube each have their own 7TV path (by platform user id);
      // Twitch keeps the full BTTV/FFZ/7TV/native fetch. TikTok has no
      // channel-emote fetch — its messages are plain text baked at parse time,
      // so there is no picker set to fetch.
      const set =
        provider === 'kick'
          ? await fetchKickChannelEmotes(channel.toLowerCase())
          : provider === 'youtube'
            ? await fetchYouTubeChannelEmotes(channel.toLowerCase())
            : provider === 'tiktok'
              ? null
              : await fetchAllEmotes(channel.toLowerCase(), channelId);
      if (set) {
        emoteCache.set(key, set);
        notifyEmoteSubscribers(key);
      }
      return set;
    } catch (err) {
      Logger.warn(`[ChatStore] ensureChannelEmotes failed for ${key}:`, err);
      return null;
    } finally {
      inflightEmoteFetches.delete(key);
    }
  })();
  inflightEmoteFetches.set(key, promise);
  return promise;
}

// --- Helpers ----------------------------------------------------------------

function bumpRevision() {
  useChatConnectionStore.setState((state) => ({ revision: state.revision + 1 }));
}

/// Global bump plus the given channels' counters, in one setState. Only the
/// paths that know their channel use this (flushPending, withSlice, slice
/// lifecycle); the ~25 no-arg bumpRevision sites keep global-only semantics,
/// which several of them (NOTICE loops, all-channel connect state) need.
function bumpRevisionFor(channelKeys: string[]) {
  useChatConnectionStore.setState((state) => {
    const next = { ...state.revisionByChannel };
    for (const key of channelKeys) {
      next[key] = (next[key] ?? 0) + 1;
    }
    return { revision: state.revision + 1, revisionByChannel: next };
  });
}

/** Every open channel's counter, for a change that touched every slice. */
function bumpAllChannels() {
  bumpRevisionFor([...useChatConnectionStore.getState().channels.keys()]);
}

/**
 * Lazily-loaded per-message engines, resolved once and then called synchronously.
 *
 * These were `import()`ed inside the per-message path, so every single chat
 * message allocated two promises and queued two microtask hops even when no nuke
 * or reminder was armed. The module registry caches the module, but not the
 * promise/closure churn. First use still loads asynchronously (so the chunk stays
 * split); every message after that takes the synchronous branch.
 */
type NukeEngine = typeof import('../utils/nukeEngine');
let nukeEngineMod: NukeEngine | null = null;
let nukeEnginePromise: Promise<NukeEngine> | null = null;

function withNukeEngine(fn: (mod: NukeEngine) => void): void {
  if (nukeEngineMod) { fn(nukeEngineMod); return; }
  nukeEnginePromise ??= import('../utils/nukeEngine');
  void nukeEnginePromise.then((mod) => { nukeEngineMod = mod; fn(mod); });
}


// --- Coalesced render flush ---------------------------------------------------
//
// Player and chat share one webview main thread, and hls.js appends to the
// video buffer from it. Rendering once per message pins that thread under fast
// chat and starves the appends, which stalls playback.
//
// New messages are queued so the array append and the render happen once per
// animation frame, bounding render rate by frame rate however fast chat moves.
//
// Dedup and the in-place reconciliation paths (own-message echo upgrade, Helix
// id stamp, moderation) still run synchronously at ingestion; only the append
// and the render are deferred. In-place paths call scheduleFlush(); new
// messages call queueMessage().
const pendingByChannel = new Map<string, any[]>();
// Two schedulers race to drain the queue, and the gate is "is any timer armed",
// never a sticky boolean.
//
// rAF is the fast path, firing at frame rate while the window is visible. The
// timeout is the liveness guarantee: rAF callbacks are suspended, and can be
// dropped outright rather than deferred, while a WebView2 window is occluded,
// minimized or mid-fullscreen-transition. A lone rAF gate whose callback was
// dropped would wedge the only live-render path with no recovery short of
// releasing the channel. The timeout caps a dropped flush at
// FLUSH_MAX_LATENCY_MS. Whichever fires first drains and cancels the other.
let rafHandle: number | null = null;
let timeoutHandle: ReturnType<typeof setTimeout> | null = null;

const FLUSH_MAX_LATENCY_MS = 250;

function runFlush(): void {
  // Disarm both schedulers and null the handles BEFORE flushing, so the next
  // queueMessage re-arms cleanly and a throw inside flushPending can never strand
  // a handle that would block every future flush.
  if (rafHandle !== null) {
    if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(rafHandle);
    rafHandle = null;
  }
  if (timeoutHandle !== null) {
    clearTimeout(timeoutHandle);
    timeoutHandle = null;
  }
  flushPending();
}

function scheduleFlush(): void {
  // Already armed — whichever timer wins drains everything queued since.
  if (rafHandle !== null || timeoutHandle !== null) return;
  // Always arm the timeout (liveness). Add the rAF fast path when visible; when
  // hidden, the timeout alone drains the queue (throttled by the platform, but
  // it always fires, so chat is current the moment the window is shown again).
  timeoutHandle = setTimeout(runFlush, FLUSH_MAX_LATENCY_MS);
  if (typeof requestAnimationFrame === 'function' && !isWindowHidden()) {
    rafHandle = requestAnimationFrame(runFlush);
  }
}

// Event retention and post-resume decay live in chatBufferTrim.ts (pure, unit-tested).

// Deletion marks (CLEARMSG) accumulate one id per moderation event for the
// life of the slice. A mark may only be dropped when it is provably inert:
// the id is still in seenMessageIds (so a backfill re-insert is DEDUPED away
// and the mark can never style anything again) while its row is gone from the
// buffer. Marks for ids NOT in the dedup set are kept - the backfill could
// re-insert those messages and they must still render moderated. Never pruned
// by age (a CLEARMSG can land seconds after its row scrolled out).
const MOD_MARK_PRUNE_THRESHOLD = 1000;
function pruneModerationMarks(slice: ChannelSlice): void {
  if (slice.deletedMessageIds.size <= MOD_MARK_PRUNE_THRESHOLD) return;
  const inBuffer = new Set<string>();
  for (const m of slice.messages) {
    const id = typeof m === 'string' ? m.match(/(?:^|;)id=([^;]+)/)?.[1] : (m as any)?.id;
    if (id) inBuffer.add(id);
  }
  const kept = new Set<string>();
  for (const id of slice.deletedMessageIds) {
    const inert = slice.seenMessageIds.has(id) && !inBuffer.has(id);
    if (!inert) kept.add(id);
  }
  slice.deletedMessageIds = kept;
}

// Amortized, backfill-safe dedup-set trim. The buffer is NOT a superset of
// recent ids (event retention keeps event rows and drops ordinary ones), and
// the post-outage backfill replays the outage window with ~30s of overlap on
// each side, deduped ONLY by these ids - so the trimmed set keeps the newest
// insertions AND every id still in the buffer, with slack so the rebuild runs
// once per ~256 messages instead of per message.
const SEEN_TRIM_SLACK = 256;
function trimSeenIds(slice: ChannelSlice): void {
  const cap = CHAT_MAX_WITH_BUFFER;
  if (slice.seenMessageIds.size <= cap + SEEN_TRIM_SLACK) return;
  const keep = new Set(Array.from(slice.seenMessageIds).slice(-cap));
  for (const m of slice.messages) {
    const id = typeof m === 'string' ? m.match(/(?:^|;)id=([^;]+)/)?.[1] : (m as any)?.id;
    if (id) keep.add(id);
  }
  slice.seenMessageIds = keep;
}

function flushPending(): void {
  const state = useChatConnectionStore.getState();
  for (const [key, queued] of pendingByChannel) {
    if (queued.length === 0) continue;
    const slice = state.channels.get(key);
    if (!slice) continue;
    if (slice.historyHold) {
      // Join hold: the rows paint with the backfill (releaseHistoryHold).
      slice.historyHold.held.push(...queued);
      continue;
    }
    const historyMax = getActiveHistoryMax();
    // After a resume the buffer can still hold up to CHAT_BUFFER_SIZE rows of
    // paused overflow. Never cut to historyMax in one step: that deletes scrollback
    // the user is mid-read of, a visible jump. Let the overflow decay a few rows
    // per flush from the top instead, invisible from the bottom they resumed to.
    // setChannelPaused only records the allowance; every live append shares
    // liveAppendLimit so no one path drains it faster. See chatBufferTrim.ts for
    // why the allowance is its own counter.
    const limit = liveAppendLimit(slice, historyMax);
    // Push everything received this frame, then trim event-aware so a burst can't
    // evict recent subs/redemptions/raids from the shared buffer. liveMessageCount
    // still counts every message (drives the accurate "N new since paused" badge).
    slice.liveMessageCount += queued.length;
    // Copy-on-write (see the helpers above pushMessage): the array identity
    // changes with its content, so consumers can memoize on it.
    slice.messages = trimWithEventRetention(slice.messages.concat(queued), limit, slice.liveMessageCount);
    pruneModerationMarks(slice);
  }
  const touched = Array.from(pendingByChannel.keys());
  pendingByChannel.clear();
  // flushPending only runs when something called scheduleFlush(), so a render is
  // always warranted (covers both new-message appends and in-place upgrades).
  // In-place upgrades ride the global counter their own callers already bump.
  bumpRevisionFor(touched);
}

// Drain any queued messages into their slices immediately, outside the scheduled
// frame. Used by paths that scan slice.messages and must see just-arrived
// messages (e.g. a CLEARCHAT computing which messages a ban affects).
function flushPendingNow(): void {
  // A caller that must see every arrived row (a CLEARCHAT computing which
  // messages it covers) ends any join hold first.
  const released: string[] = [];
  for (const s of useChatConnectionStore.getState().channels.values()) {
    if (releaseHistoryHold(s)) released.push(s.channel);
  }
  runFlush();
  if (released.length) bumpRevisionFor(released);
}

// Queue a brand-new message for the next coalesced flush instead of rendering it
// immediately. Dedup + reconciliation have already run on the caller's side.
function queueMessage(channelKey: string, msg: any): void {
  let q = pendingByChannel.get(channelKey);
  if (!q) {
    q = [];
    pendingByChannel.set(channelKey, q);
  }
  q.push(msg);
  scheduleFlush();
}

function getSlice(channel: string): ChannelSlice | undefined {
  return useChatConnectionStore.getState().channels.get(channel.toLowerCase());
}

function withSlice(channel: string, mutator: (slice: ChannelSlice) => void): void {
  const slice = getSlice(channel);
  if (!slice) return;
  mutator(slice);
  bumpRevisionFor([slice.channel]);
}

/// Seed a Kick pane with the channel's recent scrollback.
///
/// Deduped against whatever the socket delivered while the fetch was in flight,
/// and PREPENDED, because history belongs above the live rows that raced in. Same
/// reasoning as the Twitch preload path: appending would interleave stale rows
/// under live ones, and skipping the dedup would prepend a second copy of a
/// message already on screen, which React reconciles as a duplicate key.
async function seedKickHistory(key: string, channel: string): Promise<void> {
  let history: any[] = [];
  try {
    history = await invoke<any[]>('kick_chat_history', { channel });
  } catch (e) {
    Logger.warn('[ChatStore] kick_chat_history failed:', e);
    return;
  }
  if (!history.length) return;
  withSlice(key, (slice) => {
    const existingIds = new Set<string>();
    for (const m of slice.messages) {
      const eid = typeof m === 'string' ? undefined : (m as any)?.id;
      if (eid) existingIds.add(eid);
    }
    const fresh: any[] = [];
    // Chat filters (hidden users, bots, ignored phrases) are applied in Rust
    // before this history is returned (services/chat_rules.rs), so nothing
    // needs re-checking here.
    for (const msg of history) {
      const id = msg?.id;
      if (id) {
        if (slice.seenMessageIds.has(id) || existingIds.has(id)) continue;
        slice.seenMessageIds.add(id);
      }
      fresh.push(msg);
    }
    if (!fresh.length) return;
    slice.messages = [...fresh, ...slice.messages];
    const limit = getActiveHistoryMax();
    if (slice.messages.length > limit) {
      slice.messages = slice.messages.slice(slice.messages.length - limit);
    }
  });
}

function emptySlice(
  channel: string,
  channelId: string | null,
  provider: ProviderId = 'twitch',
): ChannelSlice {
  return {
    channel: channel.toLowerCase(),
    provider,
    channelId,
    messages: [],
    isConnected: false,
    error: null,
    roomState: { ...EMPTY_ROOM_STATE },
    userBadges: null,
    deletedMessageIds: new Set(),
    clearedUserContexts: new Map(),
    pinnedMessage: null,
    refCount: 0,
    isPausedForBuffer: false,
    resumeOverflow: 0,
    liveMessageCount: 0,
    seenMessageIds: new Set(),
    pendingUpgradeIds: new Set(),
    userBadgesFromIrc: null,
    userColorFromIrc: lastOwnChatColor(),
    historyHold: null,
  };
}

/** The key a slice is actually STORED under.
 *
 *  `setSlice` lowercases unconditionally, so this is the only form that can be
 *  found in `channels`. `makeKey` preserves case for YouTube, so a caller that
 *  computes a composite key and looks it up directly MISSES ITS OWN SLICE, and a
 *  missed slice means the ref count never rises and `releaseChannel` never PARTs
 *  the channel or frees its emote metadata.
 *
 *  NOT the same as the message-routing key: routing lowercases a whole composite
 *  string, this folds (provider, channel). Both land on lowercase because storage
 *  is lowercase. Do not unify them by making either side case-preserving. */
// sliceLookupKey lives in utils/providerKey (pure, testable without this store)
// and is re-exported so every existing importer keeps working.
export { sliceLookupKey };

/** Stores a slice, lowercasing the key unconditionally, which makes storage the
 *  authority on key shape: an acquireChannel key of `youtube:HVtwmO9RLNw` is
 *  stored as `youtube:hvtwmo9rlnw`. Every lookup must fold to this form. */
function setSlice(channel: string, slice: ChannelSlice) {
  const key = channel.toLowerCase();
  useChatConnectionStore.setState((state) => {
    const next = new Map(state.channels);
    next.set(key, slice);
    const rev = { ...state.revisionByChannel };
    rev[key] = (rev[key] ?? 0) + 1;
    return { channels: next, revision: state.revision + 1, revisionByChannel: rev };
  });
}

function removeSlice(channel: string) {
  const key = channel.toLowerCase();
  const gone = useChatConnectionStore.getState().channels.get(key);
  if (gone?.historyHold) {
    clearTimeout(gone.historyHold.timer);
    gone.historyHold = null;
  }
  historyInFlight.delete(key);
  useChatConnectionStore.setState((state) => {
    const next = new Map(state.channels);
    next.delete(key);
    const rev = { ...state.revisionByChannel };
    rev[key] = (rev[key] ?? 0) + 1;
    return { channels: next, revision: state.revision + 1, revisionByChannel: rev };
  });
}

// Resolve the active per-channel buffer cap. Settings can override the
// hardcoded 100 default within [50, 1000] on desktop and [50, 300] on the
// phone: a device soak measured a 1,000-row buffer at 8x the DOM, heap and
// decoded images of a 130-row run, on hardware whose whole cost is
// compositing. Out-of-range values fall back to the default rather than
// crashing.
const BUFFER_CEILING = IS_MOBILE ? 300 : 1000;
/** How many rows one channel keeps. Exported because a merged feed has to cap
 *  the COMBINED array against the same number rather than let it grow to
 *  sources x cap: the message list has no JS windowing, so three sources at a
 *  raised cap would mount thousands of rows. */
export function getActiveHistoryMax(): number {
  const setting = useAppStore.getState().settings.chat_render?.message_buffer_cap;
  if (typeof setting !== 'number' || !Number.isFinite(setting)) return CHAT_HISTORY_MAX;
  return Math.max(50, Math.min(BUFFER_CEILING, Math.round(setting)));
}

// Timestamp of a buffered message in unix ms: structured rows carry
// tmi-sent-ts millis in `timestamp`; raw IRC strings carry the tag; system
// rows injected before that field existed have neither.
function messageTs(m: any): number | null {
  if (typeof m === 'string') {
    const t = m.match(/(?:^|;)tmi-sent-ts=(\d+)/)?.[1];
    return t ? Number(t) : null;
  }
  const n = Number(m?.timestamp);
  return Number.isFinite(n) ? n : null;
}

// Insert backfill messages (ascending by timestamp, already deduped) into the
// buffer chronologically: each lands right after the last existing row whose
// timestamp is not later. Scans from the end because gap messages belong near
// it; timestamp-less rows never move and never anchor an insertion.
function insertChronological(slice: ChannelSlice, incoming: any[]): void {
  if (incoming.length === 0) return;
  const out = [...slice.messages];
  for (const msg of incoming) {
    const ts = messageTs(msg);
    let insertAt = out.length;
    if (ts !== null) {
      insertAt = 0;
      for (let i = out.length - 1; i >= 0; i--) {
        const existingTs = messageTs(out[i]);
        if (existingTs !== null && existingTs <= ts) {
          insertAt = i + 1;
          break;
        }
      }
    }
    out.splice(insertAt, 0, msg);
  }
  slice.messages = out;
}

// --- Copy-on-write for slice.messages ----------------------------------------
//
// React treats array identity as the change signal, and so does React
// Compiler's memoization. Mutating the row array in place and leaning on
// renderToken is the shape the compiler cannot see through: a compiled consumer
// caches derived values by identity and goes stale.
//
// Every write produces a new array instead: at most cap + 30 references, on
// paths that run at most once per frame. Identity is a truthful signal again;
// renderToken stays as a second one.
function replaceMessageAt(slice: ChannelSlice, index: number, msg: any): void {
  const next = slice.messages.slice();
  next[index] = msg;
  slice.messages = next;
}

function removeMessageAt(slice: ChannelSlice, index: number): void {
  const next = slice.messages.slice();
  next.splice(index, 1);
  slice.messages = next;
}

// --- Join backfill ---------------------------------------------------------
// Rust fetches and parses the recent-messages mirror in one call
// (load_channel_history, commands/chat.rs). Started at acquire time so it runs
// alongside the IRC connect instead of after it, while the slice holds the
// first live rows so history and the live tail paint together.
const HISTORY_HOLD_MS = 1500;
const historyInFlight = new Map<string, Promise<any[]>>();

interface HistoryWindow {
  limit?: number;
  afterMs?: number | null;
  beforeMs?: number | null;
}

function loadHistory(key: string, window?: HistoryWindow): Promise<any[]> {
  return invoke<any[]>('load_channel_history', {
    channel: key,
    limit: window?.limit,
    afterMs: window?.afterMs ?? undefined,
    beforeMs: window?.beforeMs ?? undefined,
  }).catch((err) => {
    Logger.warn(`[ChatStore] load_channel_history failed for ${key}:`, err);
    return [] as any[];
  });
}

function startChannelHistory(key: string): void {
  if (!historyInFlight.has(key)) historyInFlight.set(key, loadHistory(key));
}

function armHistoryHold(slice: ChannelSlice): void {
  if (slice.historyHold) return;
  const key = slice.channel;
  slice.historyHold = {
    held: [],
    timer: setTimeout(() => {
      // Mirror slower than the cap: show the live rows now. History prepends
      // when it arrives, which is the old behaviour kept as the fallback.
      const s = getSlice(key);
      if (s && releaseHistoryHold(s)) bumpRevisionFor([key]);
    }, HISTORY_HOLD_MS),
  };
}

/** Ends the join hold, appending whatever it held after the current rows.
 *  Returns whether it appended anything; callers bump the revision. */
function releaseHistoryHold(slice: ChannelSlice): boolean {
  const hold = slice.historyHold;
  if (!hold) return false;
  clearTimeout(hold.timer);
  slice.historyHold = null;
  if (hold.held.length === 0) return false;
  slice.liveMessageCount += hold.held.length;
  slice.messages = trimWithEventRetention(
    slice.messages.concat(hold.held),
    liveAppendLimit(slice, getActiveHistoryMax()),
    slice.liveMessageCount,
  );
  return true;
}

function pushMessage(slice: ChannelSlice, msg: any) {
  // An own send or a system row is something the user is looking for right
  // now: end the join hold rather than park it.
  releaseHistoryHold(slice);
  const limit = liveAppendLimit(slice, getActiveHistoryMax());
  // Monotonic — counts the append regardless of any trim below. Drives the
  // accurate "N new since paused" badge.
  slice.liveMessageCount++;
  slice.messages = trimWithEventRetention(slice.messages.concat([msg]), limit, slice.liveMessageCount);
}

/**
 * Retroactively repaint the PRIMARY account's own optimistic messages with the
 * latest USERSTATE badge set, and report whether anything changed.
 *
 * Why this is necessary: Twitch never echoes your own PRIVMSG back over your own
 * IRC read connection, so an own message exists only as the local optimistic
 * copy — the id-match echo upgrade in handleRawIrcString / appendStructuredMessage
 * can't fire for it. Its `badges=` tag is frozen at build time to
 * `slice.userBadgesFromIrc` (USERSTATE). If you send before USERSTATE has landed,
 * that tag is empty and there is otherwise NO path to your real badges short of a
 * leave + rejoin backfill (parse_historical_messages). This closes that gap by
 * rewriting the tag the moment USERSTATE arrives. Mirrors the cosmetics-repaint
 * bridge in chatUserStore, but for native Twitch badges. Only raw-string copies
 * are touched — backfilled / reconciled messages are structured objects that
 * already carry authoritative server badges.
 */
function repaintOwnBadges(slice: ChannelSlice, badges: string): boolean {
  return repaintOwnRows(slice, (m) => {
    const current = m.badges.map((b) => `${b.name}/${b.version}`).join(',');
    return current === badges ? null : { ...m, badges: badgesFromTag(badges) };
  });
}

/**
 * Retroactively repaint the PRIMARY account's own optimistic messages with the
 * real chat color from USERSTATE. Same rationale as repaintOwnBadges: Twitch
 * doesn't echo your own PRIVMSG back over your own read connection, so an own
 * message's color is frozen at build time. If it was built before USERSTATE
 * landed (or with the default fallback), this rewrites it the moment the real
 * color arrives so the username never stays a wrong color.
 */
function repaintOwnColor(slice: ChannelSlice, color: string): boolean {
  return repaintOwnRows(slice, (m) => (m.color === color ? null : { ...m, color }));
}

/** Our own not-yet-confirmed row, built by Rust at send time. */
function isOwnOptimistic(m: unknown): m is BackendChatMessage {
  return typeof m === 'object' && m !== null && (m as BackendChatMessage).own_optimistic === true;
}

/** Still carrying its provisional id (Helix has not answered yet). */
function isUnstampedOwn(m: unknown): m is BackendChatMessage {
  return isOwnOptimistic(m) && m.id.startsWith('local-');
}

function badgesFromTag(tag: string): BackendChatMessage['badges'] {
  return tag
    .split(',')
    .filter(Boolean)
    .map((b) => {
      const [name, version = ''] = b.split('/');
      return { name, version };
    });
}

/** Replace the primary account's own rows that `repaint` changes. Only rows this
 *  window built are touched: anything received carries the server's own values. */
function repaintOwnRows(
  slice: ChannelSlice,
  repaint: (m: BackendChatMessage) => BackendChatMessage | null,
): boolean {
  if (!currentUserId) return false;
  let next: any[] | null = null;
  for (let i = 0; i < slice.messages.length; i++) {
    const m = slice.messages[i];
    if (!isOwnOptimistic(m) || m.user_id !== currentUserId) continue;
    const painted = repaint(m);
    if (!painted) continue;
    next ??= slice.messages.slice();
    next[i] = painted;
  }
  if (!next) return false;
  slice.messages = next;
  return true;
}

function setAllChannelsConnected(connected: boolean) {
  for (const slice of useChatConnectionStore.getState().channels.values()) {
    slice.isConnected = connected;
  }
  // Every slice changed, and a pane subscribes to its own channel's counter,
  // so a global-only bump left the connection state stale until the next
  // message in each room.
  bumpAllChannels();
}

function setAllChannelsError(error: string | null) {
  for (const slice of useChatConnectionStore.getState().channels.values()) {
    slice.error = error;
  }
  bumpAllChannels();
}

// Extract the lowercase channel from an IRC line by locating the ` #` segment.
function extractChannelFromIrc(line: string): string | null {
  const idx = line.indexOf(' #');
  if (idx === -1) return null;
  const after = line.slice(idx + 2);
  const end = after.search(/[\s\r\n]/);
  const name = end === -1 ? after : after.slice(0, end);
  return name ? name.toLowerCase() : null;
}

// --- WebSocket lifecycle ----------------------------------------------------

async function openWebSocketWithRetry(port: number): Promise<WebSocket> {
  for (let attempt = 0; attempt < WS_OPEN_RETRY_ATTEMPTS; attempt++) {
    if (attempt > 0) {
      const delay = 500 + attempt * 500;
      Logger.debug(
        `[ChatStore] Waiting ${delay}ms before WS connection attempt ${attempt + 1}/${WS_OPEN_RETRY_ATTEMPTS}`,
      );
      await new Promise((r) => setTimeout(r, delay));
    }
    let socket: WebSocket | null = null;
    try {
      socket = new WebSocket(`ws://localhost:${port}`);
      const pending = socket;
      await new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('WS open timeout')), 5_000);
        pending.onopen = () => {
          clearTimeout(timeout);
          resolve();
        };
        pending.onerror = () => {
          clearTimeout(timeout);
          reject(new Error('WS open error'));
        };
      });
      return socket;
    } catch (err) {
      // Close the abandoned socket: one that merely timed out can still open a
      // moment later, and an unowned live socket holds a bridge client slot
      // nothing will ever read.
      if (socket) {
        socket.onopen = null;
        socket.onerror = null;
        try {
          socket.close();
        } catch {
          // ignore
        }
      }
      Logger.error(`[ChatStore] WS open attempt ${attempt + 1} failed:`, err);
      if (attempt === WS_OPEN_RETRY_ATTEMPTS - 1) throw err;
    }
  }
  throw new Error('All WS open attempts failed');
}

function startHealthCheck() {
  if (healthCheckTimer) clearInterval(healthCheckTimer);
  healthCheckTimer = setInterval(() => {
    // A dead socket is the case this watchdog exists for. It used to return
    // here whenever the socket was not OPEN, which disabled the entire ladder
    // below at exactly the moment it was needed and left chat silent forever.
    // Now: if nothing is already working on it, revive it.
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      if (useChatConnectionStore.getState().channels.size === 0) return;
      if (reconnectTimer || reconnectInFlight || wsConnectPromise) return;
      Logger.warn('[ChatStore] Health check found no live socket — scheduling reconnect');
      scheduleReconnect(0);
      return;
    }
    const elapsed = Date.now() - lastMessageTime;
    if (elapsed > STALE_WARNING_MS && elapsed <= STALE_RECONNECT_MS) {
      Logger.warn(
        `[ChatStore] No frames for ${Math.floor(elapsed / 1000)}s — connection may be stale`,
      );
      setAllChannelsError(`Connection may be stale — no data for ${Math.floor(elapsed / 1000)}s`);
    } else if (elapsed > STALE_RECONNECT_MS) {
      Logger.debug('[ChatStore] No frames for 3+ minutes — checking stream / reconnecting');
      lastMessageTime = Date.now();

      // Stage 1: an invisible probe before any teardown. The backend re-JOINs
      // its channels; a healthy connection re-acks (a ROOMSTATE frame arrives,
      // which resets lastMessageTime and clears staleNudgeAtMs), while a deaf
      // socket or lost JOIN stays silent — and only then, after a SECOND full
      // stale window, does stage 2 below reconnect/escalate. Quiet channels no
      // longer trigger teardowns, and lost JOINs recover without one.
      const hasTwitchSlice = Array.from(
        useChatConnectionStore.getState().channels.values(),
      ).some((s) => s.provider === 'twitch');
      if (hasTwitchSlice && staleNudgeAtMs === null) {
        staleNudgeAtMs = Date.now();
        Logger.warn('[ChatStore] No frames for 3m — nudging backend re-JOIN before escalating');
        invoke('nudge_chat_channels').catch(() => {
          // No IRC connection to nudge; stage 2 handles it next window.
        });
        return;
      }
      staleNudgeAtMs = null;

      const { handleStreamOffline, currentStream, isAutoSwitching } = useAppStore.getState();
      if (!currentStream || isAutoSwitching) return;
      // `check_stream_online` is Helix, so it would look up a same-named TWITCH
      // channel for a Kick/YouTube/TikTok stream and answer about the wrong
      // thing entirely. Provider streams have their own liveness poll in
      // AppStore, so here we only reconnect the socket.
      if (streamProvider(currentStream) !== 'twitch') {
        Logger.debug('[ChatStore] Provider stream: reconnecting chat without a Helix check');
        scheduleReconnect(0);
        return;
      }
      (async () => {
        try {
          // The command's argument is user_login (camelCased by Tauri); passing
          // { channel } rejects with a missing-arg error, which lands in the
          // catch below and silently turned this offline check into a chat
          // reconnect every time.
          const online = await invoke<object | null>('check_stream_online', {
            userLogin: currentStream.user_login,
          });
          if (online) {
            Logger.debug('[ChatStore] Stream online but chat dead, reconnecting chat');
            watchdogCycles++;
            if (watchdogCycles >= 2) {
              // Two watchdog reconnects without recovery: the backend task is
              // alive but wedged, and start_chat's idempotent path cannot fix
              // that. Force the one true teardown before reconnecting.
              Logger.warn(
                '[ChatStore] Watchdog escalation: stopping chat service for cold restart',
              );
              watchdogCycles = 0;
              try {
                // Recovery intent, NOT user intent: this must tear the shared WS
                // bridge down even when Kick/YouTube panes are riding it, because
                // rebuilding it is the whole point. `stop_chat` deliberately
                // preserves the bridge for those providers and so cannot recover
                // a wedged task. Provider slices come back via reconnectAll below.
                await invoke('restart_chat_bridge');
              } catch {
                // Proceed to reconnect regardless.
              }
            }
            scheduleReconnect(0);
          } else {
            Logger.debug('[ChatStore] Stream offline, triggering handleStreamOffline');
            handleStreamOffline();
          }
        } catch (err) {
          Logger.warn('[ChatStore] Stream online check failed, reconnecting anyway:', err);
          scheduleReconnect(0);
        }
      })();
    }
  }, HEALTH_CHECK_INTERVAL_MS);
}

function clearHealthCheck() {
  if (healthCheckTimer) {
    clearInterval(healthCheckTimer);
    healthCheckTimer = null;
  }
}

function scheduleReconnect(delayMs: number) {
  if (reconnectTimer) clearTimeout(reconnectTimer);
  reconnectTimer = setTimeout(() => {
    // Cleared BEFORE running, because this handle doubles as the "a reconnect
    // is already pending" signal the health check reads. Leaving a fired timer
    // set made that guard permanently true after the first reconnect, which
    // silently disabled the dead-socket watchdog for the rest of the session.
    reconnectTimer = null;
    void reconnectAll();
  }, delayMs);
}

async function reconnectAll(force = false) {
  if (reconnectInFlight) {
    // Never drop the request. Callers cannot see that this returned without
    // doing anything (scheduleReconnect discards the promise), so a reconnect
    // that lands mid-flight would be lost outright and nothing would retry.
    reconnectPending = true;
    if (force) reconnectForcePending = true;
    return;
  }
  reconnectInFlight = true;
  try {
    await reconnectAllInner();
  } finally {
    reconnectInFlight = false;
    const forced = reconnectForcePending;
    if (reconnectPending || forced) {
      reconnectPending = false;
      reconnectForcePending = false;
      // A forced request always re-runs: it wants the socket REBUILT (the
      // caller has typically already wiped the pane and is relying on the
      // reconnect to re-seed history). A plain one only re-runs if we still
      // lack a socket, so a request that arrived during a connect that
      // ultimately succeeded costs nothing.
      if (forced || !socketIsOpen()) scheduleReconnect(500);
    }
  }
}

async function reconnectAllInner() {
  const state = useChatConnectionStore.getState();
  const channels = Array.from(state.channels.keys());
  if (channels.length === 0) return;
  clearAllChannelRetries();

  intentionalDisconnect = true;
  if (ws) {
    ws.onmessage = null;
    ws.onerror = null;
    ws.onclose = null;
    ws.onopen = null;
    try {
      ws.close(1000, 'Reconnect');
    } catch {
      // ignore
    }
    ws = null;
  }
  intentionalDisconnect = false;

  // Re-attach with the first channel, then re-claim the rest. Rust records
  // consumer claims per window label in a set, so re-claiming a channel this
  // window already holds is a no-op, while re-claiming after a cold Rust
  // restart correctly re-registers us. `reattach: true` skips start_chat's
  // stale-claim sweep, which assumes a claim-starting window holds no
  // channels: true for a first acquire, not here.
  const first = channels[0];
  const firstSlice = state.channels.get(first);
  if (!firstSlice) return;

  try {
    await connectBridgeForFirstChannel(
      first,
      firstSlice.channelId,
      true,
      firstSlice.provider,
      parseKey(first).channel,
    );
    // The ladder verifies its own result. Returning without an open socket is
    // the failure that used to end reconnection silently, so treat it as an
    // error and let the catch below schedule the next attempt.
    if (!socketIsOpen()) {
      throw new Error('[ChatStore] reconnect finished without an open socket');
    }
    // The rest at once, each on its own: one platform failing or slow must not
    // hold the others back. Twitch channels share one IRC connection, so when
    // the first channel was not Twitch, the first Twitch one (re)starts it and
    // the others join after it.
    const rest = channels.slice(1).filter((ch) => state.channels.has(ch));
    const providerOf = (ch: string) => state.channels.get(ch)?.provider ?? 'twitch';
    const twitch = rest.filter((ch) => providerOf(ch) === 'twitch');
    const others = rest.filter((ch) => providerOf(ch) !== 'twitch');
    const firstIsTwitch = firstSlice.provider === 'twitch';
    const rejoinTwitch = async () => {
      if (twitch.length === 0) return;
      let joiners = twitch;
      if (!firstIsTwitch) {
        const [head, ...tail] = twitch;
        await connectChannel(head, 'twitch', head, 'start');
        joiners = tail;
      }
      await Promise.all(joiners.map((ch) => connectChannel(ch, 'twitch', ch, 'join')));
    };
    await Promise.all([
      rejoinTwitch(),
      ...others.map((ch) => connectChannel(ch, providerOf(ch), parseKey(ch).channel, 'start')),
    ]);
  } catch (err) {
    Logger.error('[ChatStore] Reconnect failed:', err);
    setAllChannelsError('Reconnection failed');
    // Never strand ws === null: the health check and visibility handlers both
    // skip that state, so without a retry here a single failed reconnect froze
    // chat until a manual refresh. Backoff caps at 30s; any successful
    // IRC_CONNECTED resets the attempt counter.
    reconnectAttempts++;
    scheduleReconnect(Math.min(30_000, 1_000 * 2 ** reconnectAttempts));
  }
}

/**
 * Force every open chat channel to tear down and reconnect. Used after switching
 * the main account: the IRC connection authenticates as the main, so it must
 * re-auth as the new identity for sends (slash-commands, IRC fallback) and
 * user-state to be correct. No-op when no channels are open.
 */
export async function reconnectAllChannels(): Promise<void> {
  await reconnectAll(true);
}

/**
 * Hard-refresh a single channel's chat — the chat-side analog of restarting the
 * stream. Wipes the visible message buffer + dedup/moderation state, busts and
 * re-fetches the channel's emote set, then forces the shared IRC bridge to tear
 * down and reconnect (which re-preloads recent history into the cleared buffer).
 *
 * Used by the overlay Refresh button and the /reload command so a refresh
 * reloads BOTH the video and chat, not just the video. The plain "reconnect
 * because the channel is unchanged" path is a deliberate no-op (see
 * useTwitchChat.connectChat), so a true refresh has to go through here.
 * No-op when the channel isn't currently acquired.
 */
export async function hardRefreshChannel(
  channel: string,
  channelId: string | null,
): Promise<void> {
  const key = channel.toLowerCase();
  const slice = useChatConnectionStore.getState().channels.get(key);
  if (!slice) return;

  // Visibly reset the channel so the reconnect repopulates it from scratch:
  // empty buffer, cleared dedup set, no lingering moderation overlays.
  // liveMessageCount resets so the "N new since paused" baseline starts clean.
  withSlice(key, (s) => {
    s.messages = [];
    s.seenMessageIds = new Set();
    s.pendingUpgradeIds = new Set();
    s.deletedMessageIds = new Set();
    s.clearedUserContexts = new Map();
    s.liveMessageCount = 0;
    s.resumeOverflow = 0;
  });
  pendingByChannel.delete(key);

  // Bust the emote cache and re-fetch. Fire-and-forget — the picker re-renders
  // via its subscription when the fresh set lands; chat doesn't block on it.
  if (channelId) void refreshChannelEmotes(key, channelId, slice?.provider ?? 'twitch');

  // Tear down + reconnect the IRC bridge. connectBridgeForFirstChannel re-runs
  // preloadChannel for the first channel, re-seeding recent history into the
  // buffer we just cleared. Forced: this pane has ALREADY been wiped, so an
  // in-flight reconnect ending with an open socket is not good enough.
  await reconnectAll(true);
}

// [ChatPerf] Instrumentation for the "chat blank for ~30s on join" hunt.
// Brackets the connect path so a single repro names where the time goes:
// start_chat (Rust IRC connect + bridge spawn), WS open, or first relayed frame.
let chatConnectStartedAt = 0;
let chatFirstFrameLogged = true;

async function connectBridgeForFirstChannel(
  channel: string,
  channelId: string | null,
  // True when re-attaching after a reconnect, when this window's store still
  // holds channels; it suppresses the Rust-side sweep of this window's stale
  // claims that a fresh first-acquire start performs (see reconnectAll).
  reattach = false,
  // Source platform. Twitch uses start_chat (its dedicated IRC bridge); other
  // providers bring up the SAME local-WS bridge via provider_chat_connect.
  provider: ProviderId = 'twitch',
  // Platform channel for non-Twitch provider_chat_connect (the bare slug, not
  // the composite slice key). Defaults to the slice key for Twitch.
  bareChannel?: string,
): Promise<void> {
  // A connect is already running. Wait it out, then decide on FACTS: if it left
  // a usable socket we are done, otherwise run our own connect. Reporting the
  // other attempt's failure as ours used to kill chat outright, because the
  // waiter is normally the NEXT channel the user opened: a teardown bumps the
  // generation, the in-flight connect correctly discards its own socket, and
  // the new channel inherited that as a failure with nothing left to retry.
  // Bounded so a pathological chain of connects cannot spin here.
  for (let waited = 0; wsConnectPromise && waited < 3; waited += 1) {
    const inflight = wsConnectPromise;
    try {
      await inflight;
    } catch {
      // Whoever started that attempt reports its own failure.
    }
    if (socketIsOpen()) return;
  }
  const attempt = connectBridgeInner(channel, channelId, reattach, provider, bareChannel);
  wsConnectPromise = attempt;
  try {
    await attempt;
  } finally {
    wsConnectPromise = null;
  }
}

async function connectBridgeInner(
  channel: string,
  channelId: string | null,
  reattach: boolean,
  provider: ProviderId,
  bareChannel?: string,
): Promise<void> {
  {
    const gen = connectGeneration;
    Logger.debug(`[ChatStore] Invoking bridge connect for ${channel} (${provider})`);
    chatConnectStartedAt = performance.now();
    chatFirstFrameLogged = false;
    // Twitch's start_chat brings the bridge up along with its IRC connection,
    // and stays first so a cold start still orders the claims it resets. Any
    // other platform, and a Twitch start that failed, takes the socket from
    // Rust alone: every other channel in the window chats over it whatever
    // this one channel's connect does, and this channel connects after it.
    let firstFailure: unknown = null;
    let port: number;
    if (provider === 'twitch') {
      try {
        port = await withTimeout(
          invoke<number>('start_chat', { channel, reattach }),
          BRIDGE_CONNECT_TIMEOUT_MS,
          'start_chat',
        );
      } catch (err) {
        firstFailure = err;
        port = await withTimeout(invoke<number>('chat_bridge_port'), BRIDGE_CONNECT_TIMEOUT_MS, 'chat_bridge_port');
      }
    } else {
      port = await withTimeout(invoke<number>('chat_bridge_port'), BRIDGE_CONNECT_TIMEOUT_MS, 'chat_bridge_port');
    }
    Logger.info(`[ChatPerf] bridge connect took ${Math.round(performance.now() - chatConnectStartedAt)}ms`);
    useChatConnectionStore.setState({ wsPort: port });

    const tBeforeWs = performance.now();
    const socket = await openWebSocketWithRetry(port);
    Logger.info(`[ChatPerf] WS bridge open took ${Math.round(performance.now() - tBeforeWs)}ms (connect total ${Math.round(performance.now() - chatConnectStartedAt)}ms)`);
    // The teardown may have run while this connect was still awaiting. Installing
    // anyway re-armed the 30s watchdog with zero channels and left an unowned
    // socket open for good; the next acquire would then overwrite `ws` and the
    // abandoned one kept feeding duplicate frames (and kept the stale-timer clock
    // fresh) with its handlers still attached.
    if (gen !== connectGeneration || useChatConnectionStore.getState().channels.size === 0) {
      Logger.debug('[ChatStore] Discarding a bridge connect that is no longer wanted');
      try {
        socket.close(1000, 'Superseded');
      } catch {
        // ignore
      }
      if (useChatConnectionStore.getState().channels.size === 0) {
        useChatConnectionStore.setState({ wsPort: null });
      }
      return;
    }
    // Never leave a previous socket attached: two clients on one bridge means
    // every frame is handled twice.
    if (ws && ws !== socket) {
      ws.onmessage = null;
      ws.onerror = null;
      ws.onclose = null;
      ws.onopen = null;
      try {
        ws.close(1000, 'Replaced');
      } catch {
        // ignore
      }
    }
    ws = socket;
    reconnectAttempts = 0;
    lastMessageTime = Date.now();

    socket.onmessage = (event) => handleWsMessage(event.data);
    socket.onerror = (err) => {
      Logger.error('[ChatStore] WS error:', err);
      setAllChannelsError('Connection error');
      setAllChannelsConnected(false);
    };
    socket.onclose = (event) => {
      Logger.debug('[ChatStore] WS closed:', event.code, event.reason);
      setAllChannelsConnected(false);
      if (intentionalDisconnect) return;
      if (useChatConnectionStore.getState().channels.size === 0) return;
      // EVERY close code reconnects, and the ladder never gives up. Gating on
      // 1006/1001 left any other code (1000, 1005, 1011) with no retry at all,
      // and the old attempt cap ended in a terminal "refresh chat" state that a
      // flaky connection reached in about four minutes, after which chat stayed
      // dead for the rest of the session. Backoff still caps at 30s, and a
      // successful open resets the counter.
      reconnectAttempts++;
      const delay = Math.min(1_000 * 2 ** (reconnectAttempts - 1), 30_000);
      Logger.debug(
        `[ChatStore] WS closed (${event.code}); scheduling reconnect attempt ${reconnectAttempts} in ${delay}ms`,
      );
      setAllChannelsError(
        reconnectAttempts <= RECONNECT_QUIET_ATTEMPTS
          ? `Connection lost — reconnecting in ${Math.round(delay / 1000)}s`
          : `Reconnecting to chat… (attempt ${reconnectAttempts})`,
      );
      scheduleReconnect(delay);
    };

    setAllChannelsConnected(true);
    startHealthCheck();

    if (provider === 'twitch') {
      // After first-channel connect, pre-load recent messages (Twitch-only: the
      // badge cache + history backfill don't apply to other providers).
      if (firstFailure === null) void preloadChannel(channel, channelId);
      else channelConnectFailed(channel, 'twitch', channel, firstFailure);
    } else {
      // Not awaited: the socket is up, and whoever waits on this connect is
      // waiting for the socket, not for one platform's answer.
      void connectChannel(channel, provider, bareChannel ?? channel, 'start');
    }
  }
}

// A channel whose own connect failed retries alone, on its own clock, while the
// window's socket and every other channel carry on. Without this the only
// retry was a reconnect of the whole window, and only ever for its first
// channel.
const CHANNEL_RETRY_BASE_MS = 30_000;
const CHANNEL_RETRY_MAX_MS = 300_000;
const channelRetryTimers = new Map<string, ReturnType<typeof setTimeout>>();
const channelRetryAttempts = new Map<string, number>();

function clearChannelRetry(key: string) {
  const timer = channelRetryTimers.get(key);
  if (timer) clearTimeout(timer);
  channelRetryTimers.delete(key);
  channelRetryAttempts.delete(key);
}

function clearAllChannelRetries() {
  for (const timer of channelRetryTimers.values()) clearTimeout(timer);
  channelRetryTimers.clear();
  channelRetryAttempts.clear();
}

/** Mark one channel's connect as failed and schedule its own retry. */
function channelConnectFailed(key: string, provider: ProviderId, bareChannel: string, err: unknown) {
  Logger.error(`[ChatStore] ${provider} connect failed for ${key}:`, err);
  withSlice(key, (s) => {
    s.isConnected = false;
    s.error = String(err);
  });
  const existing = channelRetryTimers.get(key);
  if (existing) clearTimeout(existing);
  const attempt = (channelRetryAttempts.get(key) ?? 0) + 1;
  channelRetryAttempts.set(key, attempt);
  const delay = Math.min(CHANNEL_RETRY_BASE_MS * 2 ** (attempt - 1), CHANNEL_RETRY_MAX_MS);
  channelRetryTimers.set(
    key,
    setTimeout(() => {
      channelRetryTimers.delete(key);
      // Released since, or the socket is being rebuilt: the reconnect claims
      // every channel itself.
      if (!useChatConnectionStore.getState().channels.has(key) || !socketIsOpen()) return;
      void connectChannel(key, provider, bareChannel, 'start').then((ok) => {
        const slice = useChatConnectionStore.getState().channels.get(key);
        if (ok && provider === 'twitch' && slice) void preloadChannel(key, slice.channelId);
      });
    }, delay),
  );
}

/**
 * Connect one channel's platform on this window's open socket. Its failure is
 * its own: only its pane shows it, and it retries alone. For Twitch, `start`
 * (re)starts the IRC connection when it is down and joins onto it when it is
 * up; `join` needs it up.
 */
async function connectChannel(
  key: string,
  provider: ProviderId,
  bareChannel: string,
  twitch: 'start' | 'join',
): Promise<boolean> {
  try {
    if (provider === 'twitch') {
      await withTimeout(
        twitch === 'start'
          ? invoke('start_chat', { channel: key, reattach: true })
          : invoke('join_chat_channel', { channel: key }),
        BRIDGE_CONNECT_TIMEOUT_MS,
        twitch === 'start' ? 'start_chat' : 'join_chat_channel',
      );
    } else {
      await withTimeout(
        invoke('provider_chat_connect', { provider, channel: bareChannel }),
        BRIDGE_CONNECT_TIMEOUT_MS,
        'provider_chat_connect',
      );
    }
  } catch (err) {
    channelConnectFailed(key, provider, bareChannel, err);
    return false;
  }
  channelRetryAttempts.delete(key);
  withSlice(key, (s) => {
    s.isConnected = true;
    s.error = null;
  });
  return true;
}

// Populate the Twitch badge metadata cache for a given channel. Without this,
// `parseBadges()` returns `{info:null}` and ChatMessage renders no badge image.
// Idempotent — initializeBadgeCache deduplicates on its end. Safe to call again
// when a channelId arrives late.
async function initializeBadgesForChannel(channelId: string | null): Promise<void> {
  if (!channelId) return;
  try {
    const { initializeBadgeCache } = await import('../services/twitchBadges');
    await initializeBadgeCache(channelId);
  } catch (err) {
    Logger.warn('[ChatStore] Badge cache init failed:', err);
  }
}

interface PreloadOpts {
  mode?: 'initial' | 'backfill';
  /** Backfill only: unix ms bounds of the outage window (null = unbounded). */
  afterMs?: number | null;
  beforeMs?: number | null;
}

// Backfill fetch size: assume at most ~10 messages/sec of downtime (the
// reference-client heuristic), clamped to the buffer cap and the history
// service's 800-message ceiling.
function backfillLimit(afterMs: number | null): number {
  const cap = Math.min(getActiveHistoryMax(), 800);
  if (afterMs === null) return Math.min(100, cap);
  const seconds = Math.max(1, Math.ceil((Date.now() - afterMs) / 1000));
  return Math.max(10, Math.min(seconds * 10, cap));
}

async function preloadChannel(
  channel: string,
  channelId: string | null,
  opts?: PreloadOpts,
): Promise<void> {
  if (!channelId) return;
  const mode = opts?.mode ?? 'initial';
  const key = channel.toLowerCase();
  const __t = performance.now();
  try {
    // Badges and history in parallel. The backfill used to wait behind the
    // badge cache init and then the page fetched the mirror itself; now Rust
    // fetches and parses it, and for an initial load the call already started
    // at acquire time.
    const pendingHistory =
      mode === 'backfill'
        ? loadHistory(key, {
            limit: backfillLimit(opts?.afterMs ?? null),
            afterMs: opts?.afterMs ?? null,
            beforeMs: opts?.beforeMs ?? null,
          })
        : (historyInFlight.get(key) ?? loadHistory(key));
    historyInFlight.delete(key);
    const [, parsed] = await Promise.all([initializeBadgesForChannel(channelId), pendingHistory]);
    Logger.info(`[ChatPerf] preload: badges + history ${Math.round(performance.now() - __t)}ms (${parsed.length} rows, ${mode})`);
    if (parsed.length === 0) {
      withSlice(key, (slice) => { releaseHistoryHold(slice); });
      return;
    }
    withSlice(key, (slice) => {
      const source: any[] = parsed;

      // De-dupe against messages already in the slice. preloadChannel is async
      // while the WS subscription streams live messages immediately, so anything
      // arriving in that window is already appended. Prepending naively repeats
      // its id, which React reconciles as a duplicate key.
      //
      // The second set covers own messages: one that was sent rather than
      // received carries its real Helix id but is deliberately absent from
      // seenMessageIds, so a later IRC echo can upgrade it in place.
      const existingIds = new Set<string>();
      for (const m of slice.messages) {
        const eid = typeof m === 'string' ? m.match(/(?:^|;)id=([^;]+)/)?.[1] : (m as any)?.id;
        if (eid) existingIds.add(eid);
      }
      const filtered: any[] = [];
      // Structured backfill rows arrive through Rust's parse path, where the
      // rule engine already dropped hidden users, bots and ignored phrases.
      for (const msg of source) {
        const id =
          typeof msg === 'string' ? msg.match(/(?:^|;)id=([^;]+)/)?.[1] : msg?.id;
        if (id) {
          if (slice.seenMessageIds.has(id) || existingIds.has(id)) continue;
          slice.seenMessageIds.add(id);
        }
        filtered.push(msg);
      }

      // Initial load prepends (history belongs above the live stream that
      // raced in during the fetch). A post-outage backfill inserts each gap
      // message chronologically instead, so the hole fills in place between
      // the pre-outage rows, the disconnect marker, and post-reconnect live
      // rows.
      if (mode === 'backfill') {
        insertChronological(slice, filtered);
      } else {
        slice.messages = [...filtered, ...slice.messages];
      }
      const limit = currentBufferLimit(slice, getActiveHistoryMax());
      if (slice.messages.length > limit) {
        slice.messages = slice.messages.slice(slice.messages.length - limit);
      }
      // History is in; the held live tail goes under it in the same revision.
      if (mode !== 'backfill') releaseHistoryHold(slice);
    });
  } catch (err) {
    Logger.error('[ChatStore] Failed to load recent messages:', err);
    withSlice(key, (slice) => { releaseHistoryHold(slice); });
  }
}

/** MultiChat: a Twitch pane resolves its channelId after the pane mounts (the
 *  stream-info poll runs post-render). When a channel was acquired WITHOUT an id
 *  (a Go Live seed, or a saved source stored without one), the acquire-time
 *  preload bailed — so an OFFLINE channel, which has no live messages arriving,
 *  shows an empty pane even though the core app shows its recent chat. Once the
 *  pane has the id it calls this to run the one-time recent-history backfill.
 *  preloadChannel dedups against what's already in the slice, so this is safe
 *  even if a backfill already ran. */
export async function ensureChannelHistory(
  channel: string,
  channelId: string | null,
): Promise<void> {
  if (!channelId) return;
  await preloadChannel(channel.toLowerCase(), channelId);
}

// --- Incoming message routing ----------------------------------------------

// Per-channel throttle for the no-slice drop warning (see handleWsMessage).
const NO_SLICE_WARN_INTERVAL_MS = 60_000;
const noSliceWarnedAt = new Map<string, number>();

function handleWsMessage(raw: string) {
  // Global signals first. HEARTBEAT deliberately does NOT touch
  // lastMessageTime: the backend heartbeat only proves the socket reads
  // SOMETHING (its own PONGs included), so letting it reset the stale timer
  // blinded the watchdog to a connection that was TCP-alive but delivering no
  // channel traffic. It still clears the stale-warning banner.
  if (raw === 'HEARTBEAT') {
    setAllChannelsError(null);
    return;
  }

  lastMessageTime = Date.now();
  staleNudgeAtMs = null;

  if (!chatFirstFrameLogged) {
    chatFirstFrameLogged = true;
    Logger.info(`[ChatPerf] first chat frame relayed ${Math.round(performance.now() - chatConnectStartedAt)}ms after connect start`);
  }
  if (raw === 'IRC_CONNECTED' || raw === 'RECONNECTED') {
    setAllChannelsConnected(true);
    setAllChannelsError(null);
    clearPendingLostRow();
    reconnectAttempts = 0;
    watchdogCycles = 0;
    if (backendReconnecting) {
      backendReconnecting = false;
      // Backfill anything missed during the backend outage, bounded to the
      // outage window (30s overlap margin on each side; dedup by message id
      // makes overlap harmless). Messages insert chronologically, so the gap
      // fills in place instead of stacking at the end.
      const afterMs = outageStartedAtMs !== null ? outageStartedAtMs - 30_000 : null;
      outageStartedAtMs = null;
      const beforeMs = Date.now() + 30_000;
      const { channels } = useChatConnectionStore.getState();
      for (const [key, slice] of channels) {
        if (slice.provider === 'twitch' && slice.channelId) {
          void preloadChannel(key, slice.channelId, { mode: 'backfill', afterMs, beforeMs });
        }
      }
    }
    return;
  }
  if (raw === 'IRC_CONNECT_RETRY') {
    // Pre-establishment retry: there was never a live session to lose, so no
    // row and no pane error. Critically no backendReconnecting either; that
    // flag gates the post-outage backfill, which a first connect must not
    // trigger.
    Logger.debug('[ChatStore] backend retrying initial IRC connect');
    return;
  }
  if (raw === 'IRC_RECONNECTING') {
    if (!backendReconnecting) {
      backendReconnecting = true;
      outageStartedAtMs = lastMessageTime;
      // Grace window: the supervisor usually rebuilds in a second or two and
      // the backfill fills the gap in place, so a fast recovery stays fully
      // silent. Only an outage that outlives the window prints the inline
      // marker row (once per twitch slice) and the pane error. The
      // !backendReconnecting gate means repeated frames during one outage can
      // never stack timers or rows.
      clearPendingLostRow();
      pendingLostRowTimer = setTimeout(() => {
        pendingLostRowTimer = null;
        if (!backendReconnecting) return;
        const { channels } = useChatConnectionStore.getState();
        for (const [key, slice] of channels) {
          if (slice.provider === 'twitch') {
            injectSystemMessage(key, 'Chat connection lost, reconnecting...');
          }
        }
        setAllChannelsError('Reconnecting to chat...');
      }, LOST_ROW_GRACE_MS);
    }
    return;
  }
  if (raw.startsWith('RECONNECTING:')) {
    setAllChannelsConnected(false);
    return;
  }
  if (raw.startsWith('RECONNECT_FAILED:')) {
    return;
  }
  if (raw === 'RECONNECT_STOPPED' || raw === 'RECONNECT_EXHAUSTED') {
    setAllChannelsError(
      raw === 'RECONNECT_EXHAUSTED'
        ? 'Unable to reconnect to chat. Please refresh.'
        : 'Connection stopped',
    );
    setAllChannelsConnected(false);
    return;
  }
  if (raw.startsWith('CONNECTION_WARNING:')) {
    const warn = raw.slice('CONNECTION_WARNING:'.length);
    setAllChannelsError(`Warning: ${warn}`);
    return;
  }

  // USER_BADGES:#<channel>:<badges>  (legacy: USER_BADGES:<badges>)
  if (raw.startsWith('USER_BADGES:')) {
    const payload = raw.slice('USER_BADGES:'.length);
    let channel: string | null = null;
    let badges: string;
    if (payload.startsWith('#')) {
      const colonIdx = payload.indexOf(':');
      if (colonIdx > 1) {
        channel = payload.slice(1, colonIdx).toLowerCase();
        badges = payload.slice(colonIdx + 1);
      } else {
        badges = payload;
      }
    } else {
      badges = payload;
    }
    if (channel) {
      withSlice(channel, (slice) => {
        slice.userBadgesFromIrc = badges;
        slice.userBadges = badges;
        // Repaint any already-sent own messages that were built before this
        // USERSTATE landed (withSlice bumps the render revision for us).
        repaintOwnBadges(slice, badges);
      });
    } else {
      // Legacy untagged badges — apply to whichever channel exists (only one
      // when running the pre-multi-channel main app shape). Multi-channel
      // builds always carry the tag.
      const channels = useChatConnectionStore.getState().channels;
      if (channels.size === 1) {
        const slice = channels.values().next().value as ChannelSlice;
        slice.userBadgesFromIrc = badges;
        slice.userBadges = badges;
        repaintOwnBadges(slice, badges);
        bumpRevision();
      }
    }
    return;
  }

  // USER_COLOR:#<channel>:<color>  (legacy: USER_COLOR:<color>)
  // The connected user's own chat color from USERSTATE. Cache it and repaint
  // any own messages already sent with the build-time default.
  if (raw.startsWith('USER_COLOR:')) {
    const payload = raw.slice('USER_COLOR:'.length);
    let channel: string | null = null;
    let color: string;
    if (payload.startsWith('#')) {
      const colonIdx = payload.indexOf(':');
      if (colonIdx > 1) {
        channel = payload.slice(1, colonIdx).toLowerCase();
        color = payload.slice(colonIdx + 1);
      } else {
        color = payload;
      }
    } else {
      color = payload;
    }
    if (color) {
      persistOwnChatColor(color);
      if (channel) {
        withSlice(channel, (slice) => {
          slice.userColorFromIrc = color;
          repaintOwnColor(slice, color);
        });
      } else {
        const channels = useChatConnectionStore.getState().channels;
        if (channels.size === 1) {
          const slice = channels.values().next().value as ChannelSlice;
          slice.userColorFromIrc = color;
          repaintOwnColor(slice, color);
          bumpRevision();
        }
      }
    }
    return;
  }

  // Channel-tagged JSON events
  if (raw.startsWith('{')) {
    try {
      const parsed = JSON.parse(raw);
      if (parsed.type === 'CLEARMSG' && parsed.target_msg_id) {
        const ch = (parsed.channel as string | undefined)?.toLowerCase();
        const modSettings = useAppStore.getState().settings.moderation;
        const ignoreClear = modSettings?.ignore_clear_chat ?? false;
        const showModMsgs = modSettings?.show_mod_messages ?? false;
        const apply = (slice: ChannelSlice) => {
          if (!ignoreClear) slice.deletedMessageIds = new Set(slice.deletedMessageIds).add(parsed.target_msg_id);
        };
        if (ch) withSlice(ch, apply);
        else for (const s of useChatConnectionStore.getState().channels.values()) apply(s);
        // The frame may not carry the author/text (Kick's delete event only gives
        // the message id), so recover them from chat history by that id — the
        // message is still in the slice (deletion only marks it, doesn't drop it).
        // Twitch's IRC CLEARMSG already includes both, so this only fills the gaps.
        let delLogin = (parsed.login as string | undefined) || undefined;
        let delMessage = (parsed.message as string | undefined) || undefined;
        if ((!delLogin || !delMessage) && ch) {
          const hit = useChatConnectionStore
            .getState()
            .channels.get(ch)
            ?.messages.find((m) => typeof m !== 'string' && m.id === parsed.target_msg_id);
          if (hit && typeof hit !== 'string') {
            delLogin = delLogin || hit.display_name || hit.username;
            delMessage = delMessage || hit.content;
          }
        }
        if (showModMsgs && ch) {
          const who = delLogin ?? 'a user';
          // Kick's auto-mod deletes name the rule that was violated; saying so
          // beats "by a moderator" when no human was involved.
          const why = (parsed.reason as string) || '';
          injectSystemMessage(
            ch,
            why
              ? `${who}'s message was deleted (${why}).`
              : `${who}'s message was deleted by a moderator.`,
            undefined,
            systemSourceFor(ch),
          );
        }
        // Moderator log: message deletions are broadcast to every viewer over IRC,
        // so this populates the log even when you're not a mod. The EventSub feed
        // (mod-only) upgrades this entry in place with the acting moderator's name.
        useAppStore.getState().addModLog({
          id: `irc-${Date.now()}-${Math.random()}`,
          action: 'delete',
          timestamp: new Date().toISOString(),
          moderator_name: (parsed.moderator as string) || 'A moderator',
          target_user_name: delLogin || undefined,
          target_user_login: delLogin || undefined,
          message: delMessage || undefined,
          reason: (parsed.reason as string) || undefined,
          channel: ch,
          source: 'irc',
          details: parsed,
        });
        bumpRevision();
        return;
      }
      if (parsed.type === 'CLEARCHAT') {
        // Drain queued messages first so the affected-message scan below sees
        // anything that arrived in the current (not-yet-flushed) frame.
        flushPendingNow();
        const ch = (parsed.channel as string | undefined)?.toLowerCase();
        const modSettings = useAppStore.getState().settings.moderation;
        const ignoreClear = modSettings?.ignore_clear_chat ?? false;
        const showModMsgs = modSettings?.show_mod_messages ?? false;
        const apply = (slice: ChannelSlice) => {
          if (!parsed.target_user_id) return; // full chat clear — UI doesn't track this today
          if (ignoreClear) return; // user opted out of moderation strikethrough overlays
          const modType: 'timeout' | 'ban' =
            parsed.ban_duration !== undefined && parsed.ban_duration !== null
              ? 'timeout'
              : 'ban';
          const affected = new Set<string>();
          for (const msg of slice.messages) {
            const msgUserId =
              typeof msg !== 'string'
                ? msg.user_id
                : msg.match?.(/(?:^@|;)user-id=([^;]+)/)?.[1];
            const msgId =
              typeof msg !== 'string' ? msg.id : msg.match?.(/(?:^|;)id=([^;]+)/)?.[1];
            if (msgUserId === parsed.target_user_id && msgId) affected.add(msgId);
          }
          slice.clearedUserContexts = new Map(slice.clearedUserContexts).set(parsed.target_user_id, {
            context: {
              type: modType,
              duration: parsed.ban_duration,
              username: parsed.target_user,
            },
            affectedMessageIds: affected,
          });
        };
        if (ch) withSlice(ch, apply);
        else for (const s of useChatConnectionStore.getState().channels.values()) apply(s);
        if (showModMsgs && ch && parsed.target_user_id) {
          const who = parsed.target_user ?? 'A user';
          let line: string;
          if (parsed.ban_duration !== undefined && parsed.ban_duration !== null) {
            const secs = parsed.ban_duration as number;
            const human =
              secs >= 86400 ? `${Math.round(secs / 86400)}d` :
              secs >= 3600  ? `${Math.round(secs / 3600)}h` :
              secs >= 60    ? `${Math.round(secs / 60)}m` :
              `${secs}s`;
            line = `${who} was timed out for ${human}.`;
          } else {
            line = `${who} was banned.`;
          }
          injectSystemMessage(ch, line);
        } else if (showModMsgs && ch && !parsed.target_user_id) {
          injectSystemMessage(ch, 'Chat was cleared by a moderator.');
        }
        // Moderator log: timeouts/bans/clears are broadcast to every viewer over
        // IRC (anonymized — no acting moderator). This is the baseline feed that
        // works in any channel, live or offline, main or multi. The EventSub
        // channel.moderate feed upgrades these with the moderator name when you
        // moderate the channel.
        {
          const appState = useAppStore.getState();
          if (parsed.target_user_id) {
            const isTimeout = parsed.ban_duration !== undefined && parsed.ban_duration !== null;
            // Surface the target's most recent message as the likely reason,
            // mirroring how deletions show the removed text. CLEARCHAT carries no
            // message, so read it back from history: CLEARCHAT only marks messages
            // cleared, it does not drop them. Chronological order means the last
            // match wins. Also recovers what the frame omits: display name
            // (YouTube/Kick give only an id) and cleared count.
            let lastMessage: string | undefined;
            let recoveredName: string | undefined;
            let removedCount = 0;
            const targetSlice = ch ? useChatConnectionStore.getState().channels.get(ch) : undefined;
            if (targetSlice) {
              for (const msg of targetSlice.messages) {
                const msgUserId =
                  typeof msg !== 'string'
                    ? msg.user_id
                    : msg.match?.(/(?:^@|;)user-id=([^;]+)/)?.[1];
                if (msgUserId !== parsed.target_user_id) continue;
                removedCount += 1;
                if (typeof msg !== 'string') {
                  recoveredName = msg.display_name || msg.username || recoveredName;
                }
                const text =
                  typeof msg !== 'string'
                    ? (msg.content as string | undefined)
                    : msg.match?.(/PRIVMSG #\w+ :(.+)$/)?.[1];
                if (typeof text === 'string' && text.trim()) lastMessage = text.trim();
              }
            }
            appState.addModLog({
              id: `irc-${Date.now()}-${Math.random()}`,
              // YouTube's anonymous feed can't distinguish a timeout from a permanent
              // ban (both are a duration-less "remove all by author"), so log those as
              // a neutral "removed" rather than mislabeling them "banned".
              action: isTimeout ? 'timeout' : parsed.provider === 'youtube' ? 'removed' : 'ban',
              timestamp: new Date().toISOString(),
              moderator_name: (parsed.moderator as string) || 'A moderator',
              target_user_name: (parsed.target_user as string) || recoveredName || undefined,
              target_user_id: (parsed.target_user_id as string) || undefined,
              target_user_login: (parsed.target_user as string) || undefined,
              duration: isTimeout ? (parsed.ban_duration as number) : undefined,
              removed_count: removedCount || undefined,
              message: lastMessage,
              channel: ch,
              source: 'irc',
              details: parsed,
            });
          } else {
            appState.addModLog({
              id: `irc-${Date.now()}-${Math.random()}`,
              action: 'clear',
              timestamp: new Date().toISOString(),
              moderator_name: (parsed.moderator as string) || 'A moderator',
              channel: ch,
              source: 'irc',
              details: parsed,
            });
          }
        }
        bumpRevision();
        return;
      }
      if (parsed.type === 'PINNED' || parsed.type === 'UNPINNED') {
        // Provider-driven pinned message (e.g. Kick's Pusher pin event). Stash it
        // on the slice; ChatWidget feeds it into the same pinned banner as Twitch.
        const ch = (parsed.channel as string | undefined)?.toLowerCase();
        const pin = parsed.type === 'PINNED' ? parsed.pin : null;
        // withSlice already bumps when it finds the slice, and nothing changed
        // when it doesn't, so no second bump here.
        if (ch) withSlice(ch, (slice) => { slice.pinnedMessage = pin; });
        return;
      }
      if (parsed.type === 'ROOMSTATE') {
        const ch = (parsed.channel as string | undefined)?.toLowerCase();
        const apply = (slice: ChannelSlice) => {
          slice.roomState = {
            followersOnly: parsed.followers_only ?? slice.roomState.followersOnly,
            slow: parsed.slow ?? slice.roomState.slow,
            subsOnly: parsed.subs_only ?? slice.roomState.subsOnly,
            emoteOnly: parsed.emote_only ?? slice.roomState.emoteOnly,
            r9k: parsed.r9k ?? slice.roomState.r9k,
          };
        };
        if (ch) {
          // withSlice bumps for us.
          withSlice(ch, apply);
        } else {
          // The channel-less form mutates every slice directly, so it has to
          // bump itself.
          for (const s of useChatConnectionStore.getState().channels.values()) apply(s);
          bumpRevision();
        }
        return;
      }
      if (parsed.type === 'NOTICE') {
        handleNotice(parsed);
        return;
      }

      // Structured ChatMessage (from Rust parser) — route by parsed.channel if
      // present (future server change), else by content.
      const messageId = parsed.id;
      if (messageId) {
        // Determine target channel: ChatMessage carries no explicit channel
        // field today, but the channel was used at parse time. Fall back to
        // the only acquired channel when ambiguous. For multi-channel
        // operation we may want Rust to emit a channel field on ChatMessage —
        // tracked as a follow-up.
        const channels = useChatConnectionStore.getState().channels;
        let targetChannel: string | null = null;
        if (parsed.channel) {
          // Lowercased on purpose. This is NOT the case-preserving key space
          // makeKey builds: `setSlice` lowercases at the storage boundary, so
          // EVERY key in `channels` is lowercase no matter what the caller
          // computed. acquireChannel builds youtube:HVtwmO9RLNw and setSlice
          // stores it as youtube:hvtwmo9rlnw. Routing with the case-preserving
          // key therefore matches nothing and silently drops every YouTube row:
          // measured 74 drops in 90s with a dead chat pane, versus zero after
          // restoring this line. Verified on device 2026-08-29.
          targetChannel = (parsed.channel as string).toLowerCase();
        } else if (channels.size === 1) {
          targetChannel = channels.keys().next().value as string;
        }
        if (!targetChannel) {
          Logger.warn(
            `[ChatStore] Dropping structured message: no routable channel (id=${messageId}, channel=${parsed.channel ?? '∅'}, slices=${channels.size})`,
          );
          return;
        }
        const slice = channels.get(targetChannel);
        if (!slice) {
          // Expected briefly after a switch: the slice is removed before the
          // IRC PART lands, so a busy channel delivers a dozen more messages
          // into nothing. One warn per channel per minute keeps a genuine
          // routing fault visible (the keys that DO exist are listed for it)
          // without writing a line per message (67 in one instrumented run).
          const now = Date.now();
          const last = noSliceWarnedAt.get(targetChannel) ?? 0;
          if (now - last > NO_SLICE_WARN_INTERVAL_MS) {
            noSliceWarnedAt.set(targetChannel, now);
            Logger.warn(
              `[ChatStore] Dropping structured message: no slice for "${targetChannel}" (id=${messageId}, slices=${channels.size}, have=[${Array.from(channels.keys()).join(', ')}]); further drops for this channel are silent for a minute`,
            );
          }
          return;
        }
        appendStructuredMessage(slice, parsed);
        return;
      }
    } catch (e) {
      Logger.error('[ChatStore] Failed to parse JSON message:', e);
      // fall through to raw-string handling
    }
  }

  // Raw IRC string — route by the `#channel` in the line.
  handleRawIrcString(raw);
}

function handleNotice(parsed: any) {
  const msgId = parsed.msg_id as string | undefined;
  const modActionMap: Record<string, string> = {
    host_on: 'host',
    host_off: 'unhost',
    slow_on: 'slow_mode_on',
    slow_off: 'slow_mode_off',
    subs_on: 'subscriber_only_on',
    subs_off: 'subscriber_only_off',
    emote_only_on: 'emote_only_on',
    emote_only_off: 'emote_only_off',
    followers_on: 'follower_only_on',
    followers_off: 'follower_only_off',
    followers_on_zero: 'follower_only_on',
    // timeout_success / ban_success intentionally omitted: those self-action
    // NOTICEs only fire for the moderator who acted and carry no target, while
    // CLEARCHAT now logs every timeout/ban universally with the real target.
    unban_success: 'unban',
    untimeout_success: 'untimeout',
    clear_chat: 'clear_chat',
  };

  if (msgId && modActionMap[msgId]) {
    const appState = useAppStore.getState();
    const eventSubAction = msgId.replace('_on', '').replace('_off', '').replace('_success', '');
    const recentlyAdded = appState.modLogs.some(
      (l) =>
        (l.action === eventSubAction || l.action === modActionMap[msgId]) &&
        new Date(l.timestamp).getTime() > Date.now() - 2_000,
    );
    if (!recentlyAdded) {
      appState.addModLog({
        id: `irc-${Date.now()}-${Math.random()}`,
        action: modActionMap[msgId],
        timestamp: new Date().toISOString(),
        moderator_name: 'Twitch System',
        target_user_name: 'Stream/Settings',
        reason: parsed.message,
        channel: parsed.channel,
        source: 'irc',
        details: parsed,
      });
    }
  }

  // Rejection: drop the most recent optimistic message within last 5s.
  const rejectionIds = new Set([
    'msg_followersonly',
    'msg_followersonly_followed',
    'msg_followersonly_zero',
    'msg_subsonly',
    'msg_slowmode',
    'msg_r9k',
    'msg_verified_email',
    'msg_ratelimit',
    'msg_duplicate',
    'msg_banned',
    'msg_timedout',
    'msg_rejected',
    'msg_rejected_mandatory',
    'msg_requires_verified_phone_number',
  ]);
  if (msgId && rejectionIds.has(msgId)) {
    const cutoff = Date.now() - 5_000;
    for (const slice of useChatConnectionStore.getState().channels.values()) {
      for (let i = slice.messages.length - 1; i >= 0; i--) {
        const m = slice.messages[i];
        if (!isUnstampedOwn(m)) continue;
        const ts = Number(m.timestamp) || 0;
        if (ts >= cutoff) {
          removeMessageAt(slice, i);
          break;
        }
      }
    }
    bumpRevision();
  }

  // Surface notice as an inline system message in the channel it belongs to.
  // NOTICE JSON doesn't currently carry channel; for multi-channel correctness
  // we route to the only acquired channel (today's main-app shape) or skip.
  if (parsed.message) {
    const sysMsgId = `notice-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
    const channels = useChatConnectionStore.getState().channels;
    if (channels.size === 1) {
      const slice = channels.values().next().value as ChannelSlice;
      pushMessage(slice, {
        id: sysMsgId,
        username: 'System',
        display_name: 'Twitch',
        color: '#9147ff',
        badges: [{ key: 'staff/1', info: {} }],
        content: parsed.message,
        segments: [{ type: 'text', content: parsed.message }],
        is_action: false,
        is_first_message: false,
        is_mentioned: false,
        is_from_shared_chat: false,
        tags: new Map([
          ['user-id', 'tw-system'],
          ['id', sysMsgId],
        ]),
      });
      slice.seenMessageIds.add(sysMsgId);
      slice.error = parsed.message;
      bumpRevisionFor([slice.channel]);
      // Auto-clear the surfaced error after 3s
      setTimeout(() => {
        withSlice(slice.channel, (s) => {
          if (s.error === parsed.message) s.error = null;
        });
      }, 3_000);
    }
  }
}

function appendStructuredMessage(slice: ChannelSlice, parsed: any) {
  const messageId = parsed.id;
  if (!messageId) return;
  if (slice.seenMessageIds.has(messageId)) return;

  // Chat filters (hidden users, bots, ignored phrases) run in Rust before a
  // structured message is broadcast (services/chat_rules.rs): a filtered row
  // never reaches this store, so every consumer inherits the filter for free.

  // Deterministic own-message upgrade: if we already hold a (string) copy with
  // this exact id — our own optimistic message stamped with the real Helix id,
  // now awaiting its full echo — replace it in place so it picks up real
  // badges/tenure. Only own stamped messages pre-exist with a server id, so this
  // never matches a fresh incoming message.
  if (slice.pendingUpgradeIds.has(messageId)) {
    slice.pendingUpgradeIds.delete(messageId);
    const idMatchIdx = slice.messages.findIndex((m) =>
      typeof m === 'string'
        ? m.match(/(?:^@|;)id=([^;]+)/)?.[1] === messageId
        : isOwnOptimistic(m) && m.id === messageId,
    );
    if (idMatchIdx !== -1) {
      replaceMessageAt(slice, idMatchIdx, parsed);
      slice.seenMessageIds.add(messageId);
      scheduleFlush();
      return;
    }
    // Miss: the content-match reconciliation consumed the optimistic row
    // before this scan ran; fall through to the normal append path.
  }

  // Badge cache tracks only the PRIMARY (the IRC-connected account).
  if (parsed.user_id === currentUserId && Array.isArray(parsed.badges)) {
    slice.userBadgesFromIrc = parsed.badges
      .map((b: any) => `${b.name}/${b.version}`)
      .join(',');
  }

  // Replace an optimistic local-* copy of a message WE sent (from the primary OR
  // a linked secondary) when the content matches. This covers the race where a
  // secondary's IRC echo arrives before its Helix id is stamped onto the
  // optimistic; without it, the echo (a non-primary user-id) would be pushed as
  // a duplicate.
  if (isOwnUserId(parsed.user_id)) {
    // Whitespace-tolerant on both sides: the server never echoes trailing
    // whitespace, and callers other than sendChannelMessage may still hand
    // us an untrimmed optimistic line.
    const optimisticIdx = slice.messages.findIndex(
      (m) => isUnstampedOwn(m) && sameSentContent(m.content, parsed.content),
    );
    if (optimisticIdx !== -1) {
      replaceMessageAt(slice, optimisticIdx, parsed);
      slice.seenMessageIds.add(messageId);
      scheduleFlush();
      return;
    }
  }
  slice.seenMessageIds.add(messageId);
  // Cap the dedup set on the structured (production) path too (amortized,
  // backfill-safe - see trimSeenIds).
  trimSeenIds(slice);
  // Gift-bomb collapse: keep only the announcement row and fold the individual
  // gifts into its recipient list. Handles anon variants and out-of-order arrival
  // (children before their announcement), mirroring the overlay via the shared
  // matchers. A child is collapsed only once its announcement has been seen, so a
  // lone single gift still renders as its own card.
  let giftBombChildSuppressed = false;
  if (parsed.provider === 'twitch' && (useAppStore.getState().settings.collapse_gift_subs ?? true)) {
    const t = (parsed.tags ?? {}) as Record<string, string>;
    const mt = parsed.metadata?.msg_type || t['msg-id'];
    const origin = giftBombOriginOf(t);
    if (origin && isGiftBombAnnouncement(mt)) {
      // Track the announcement so its children collapse (bounded by count), seed
      // the card, and reclaim any children that arrived ahead of it.
      announcedGiftBombOrigins.add(origin);
      if (announcedGiftBombOrigins.size > MAX_TRACKED_BOMB_ORIGINS) {
        const oldest = announcedGiftBombOrigins.values().next().value;
        if (oldest !== undefined) announcedGiftBombOrigins.delete(oldest);
      }
      const n = parseInt(t['msg-param-mass-gift-count'] ?? '', 10);
      useGiftBombStore.getState().noteAnnouncement(origin, Number.isFinite(n) ? n : undefined);
      if (foldBufferedGiftChildren(slice, origin) > 0) bumpRevisionFor([slice.channel]);
    } else if (origin && isGiftBombChild(mt) && announcedGiftBombOrigins.has(origin)) {
      const rid = t['msg-param-recipient-id'] || '';
      if (rid) {
        useGiftBombStore.getState().addRecipient(origin, {
          userId: rid,
          userName: t['msg-param-recipient-user-name'] || '',
          displayName: t['msg-param-recipient-display-name'] || t['msg-param-recipient-user-name'] || '',
        });
      }
      giftBombChildSuppressed = true;
    }
  }

  // Repeat collapse: fold a run of the same message into the first one's
  // counter. Cross-user by design — the noisy case is many people posting one
  // thing, not one person repeating (Twitch already rejects that).
  let repeatSuppressed = false;
  if (slice.channel) {
    // Counts every message that gets this far, events included, so the
    // "has the anchor been trimmed yet" distance below can't undercount.
    const chKey = slice.channel.toLowerCase();
    const seq = (channelPushSeq.get(chKey) ?? 0) + 1;
    channelPushSeq.set(chKey, seq);

    const rp = useAppStore.getState().settings.message_repeat;
    // Opt-in: a user who never touched the setting gets every message, like
    // Twitch's own chat. Folding cross-user spam is a choice, not a default.
    const mode = rp?.mode ?? 'off';
    // Moderators need every message actionable, so runs stay expanded in
    // channels they moderate unless they opt out.
    const moderatingHere = (rp?.keep_all_when_moderator ?? true) && isModeratorOfSlice(slice);
    const privileged = (rp?.exempt_privileged ?? true) && isPrivilegedChatter(parsed.badges);
    // A first-time chatter's message is high-signal for the streamer and is
    // typically a generic greeting, exactly what matches an open repeat run;
    // never fold it away. Tag first, metadata fallback (backfill and future
    // paths may deliver one without the other). Tags here are a plain object,
    // not the Map ChatMessage sees.
    const rawTags = (parsed.tags ?? {}) as Record<string, string>;
    const firstTimeChatter =
      rawTags['first-msg'] === '1' || parsed.metadata?.is_first_message === true;

    if (
      mode !== 'off' &&
      !giftBombChildSuppressed &&
      !parsed.metadata?.msg_type &&
      !moderatingHere &&
      !privileged &&
      !firstTimeChatter
    ) {
      const key = normalizeForRepeat(parsed.content ?? '', rp?.match ?? 'normalized');
      if (key) {
        const windowMs = Math.max(1, rp?.window_seconds ?? 60) * 1000;
        const now = Date.now();
        let runs = openRepeatRuns.get(chKey);
        if (!runs) {
          runs = new Map();
          openRepeatRuns.set(chKey, runs);
        }
        pruneRepeatRuns(runs, now, windowMs);

        // The anchor has scrolled out once more than a full buffer's worth of
        // messages have been pushed since it landed. A run whose anchor is gone
        // must not swallow copies, or they'd disappear with nothing carrying
        // their count.
        const bufferCap = getActiveHistoryMax() + CHAT_BUFFER_SIZE;
        const existing = runs.get(key);
        const anchorLive = !!existing && seq - existing.pushSeq < bufferCap;

        if (existing && anchorLive && now - existing.atMs <= windowMs) {
          existing.count += 1;
          existing.atMs = now;
          if (existing.participants.length < 20) {
            existing.participants.push({
              userId: parsed.user_id,
              displayName: parsed.display_name || parsed.username,
            });
          }
          // Collapse folds the count onto the first row and hides this one.
          // Label leaves every row on screen and numbers THIS one, so the
          // chat reads x2, x3, x4 going down.
          const rowId = mode === 'collapse' ? existing.anchorId : messageId;
          useMessageRepeatStore
            .getState()
            .noteRun(rowId, existing.count, existing.participants);
          // The threshold ("show the count from N copies") gates the FOLD,
          // not just the badge: copies below it render as their own rows, and
          // hiding starts only once the run is big enough to earn its counter.
          // Mirrors ChatMessage's display gate exactly; below-threshold copies
          // still count into the run so the anchor's badge is the true total.
          const threshold = Math.max(2, rp?.threshold ?? 2);
          repeatSuppressed = mode === 'collapse' && existing.count >= threshold;
        } else {
          runs.set(key, {
            anchorId: messageId,
            atMs: now,
            pushSeq: seq,
            count: 1,
            participants: [],
          });
        }
      }
    }
  }

  // TikTok likes are high-frequency engagement, not conversation. Keep them OUT of
  // the chat feed (they'd bury real chat) but still feed the activity panel below
  // (the producer reads `parsed` directly, not the slice, so skipping the queue is
  // safe). Follows / gifts stay inline like every other platform's events.
  const activityOnly =
    (parsed.provider === 'tiktok' && parsed.metadata?.msg_type === 'tiktok_like') ||
    giftBombChildSuppressed ||
    repeatSuppressed;

  if (!activityOnly) {
    queueMessage(slice.channel, parsed);
  }

  // Side effects run on every real chat message, including copies that repeat
  // collapse folded out of the view. A copypasta wave is exactly what /nuke
  // targets, so hiding copies from the engine would leave it actioning only the
  // first one; a keyword reminder should fire on a folded message too.
  // Gift-bomb children and TikTok likes are events, not chat, so they stay out.
  if (slice.channel && !giftBombChildSuppressed && parsed.metadata?.msg_type !== 'tiktok_like') {
    // No-op if no nukes are armed for this channel. Fire-and-forget; nuke
    // action errors are logged inside the engine.
    withNukeEngine((mod) => {
      void mod.checkActiveNukesForMessage(slice.channel, parsed);
    });

    // Keyword reminders are matched in Rust (services/reminder_service.rs).
  }

  // Mirror non-chat channel events (subs, gifts, ... and future follows/raids/
  // hosts) into the MultiChat activity panel. Only synthesized event messages
  // carry a `msg_type`, so normal chat skips this. Every provider (Twitch
  // USERNOTICE included) is parsed to a structured ChatMessage and lands here, so
  // this single path covers them all; the raw-IRC sub producer only fires on the
  // rare parse-failure fallback, so the two never both fire for one message.
  const pk = slice.channel ? parseKey(slice.channel) : null;
  if (pk) {
    const tags = (parsed.tags ?? {}) as Record<string, string>;
    const channelKey = makeKey(pk.provider, pk.channel);
    const eventMsgType = parsed.metadata?.msg_type;
    if (eventMsgType) {
      // Sub detail lives in the USERNOTICE msg-param tags (Twitch). Kick events
      // don't carry these, so they come through undefined and the row just omits
      // them. streak-months is "0" when the subber doesn't share it.
      const num = (v: string | undefined) => {
        const n = parseInt(v ?? '', 10);
        return Number.isFinite(n) ? n : undefined;
      };
      const cumulative = num(tags['msg-param-cumulative-months']);
      const streak = num(tags['msg-param-streak-months']);
      // Community gift bombs: the `submysterygift` carries the batch size, and
      // both it and its individual `subgift` follow-ups share an origin id, so
      // the normalizer can collapse the bunch into one "gifted N subs".
      const giftCount = num(tags['msg-param-mass-gift-count']);
      const originId = tags['msg-param-origin-id'] || tags['msg-param-community-gift-id'] || undefined;
      // YouTube Super Chat detail (stamped by the youtube adapter): amount + currency
      // drive the value pill, the comment shows as the row message.
      const scAmount = (() => {
        const n = parseFloat(tags['sc-amount'] ?? '');
        return Number.isFinite(n) ? n : undefined;
      })();
      // TikTok event detail (stamped by the tiktok adapter): gift name/count feed the
      // gift row, like count feeds the hearts pill.
      const ttGiftName = tags['tt-gift-name'] || undefined;
      const ttGiftCount = num(tags['tt-gift-count']);
      const ttGiftImage = tags['tt-gift-image'] || undefined;
      const ttGiftDiamonds = num(tags['tt-gift-diamonds']);
      const ttLikeCount = num(tags['tt-like-count']);
      // The chatter's avatar rides every TikTok/YouTube event message; show it on
      // the activity row (no per-row fetch).
      const actorAvatar = tags['avatar'] || undefined;
      // The stored message's badges are the RAW backend shape ({name, version},
      // no urls). Resolve them like chat does: Twitch via the badge cache (needs
      // the channel room-id), other providers via their baked image urls.
      let badges: { key: string; info: unknown }[] | undefined;
      if (Array.isArray(parsed.badges) && parsed.badges.length > 0) {
        if (pk.provider === 'twitch') {
          const badgeStr = parsed.badges
            .map((b: { name: string; version: string }) => `${b.name}/${b.version}`)
            .join(',');
          badges = parseBadges(badgeStr, tags['source-room-id'] || tags['room-id']);
        } else {
          badges = parsed.badges
            .filter((b: { image_url_1x?: string }) => b.image_url_1x)
            .map((b: { name: string; version: string; image_url_1x?: string; title?: string }) => ({
              key: `${b.name}/${b.version}`,
              info: { image_url_1x: b.image_url_1x, image_url_2x: b.image_url_1x, title: b.title },
            }));
        }
      }
      window.dispatchEvent(
        new CustomEvent('provider-activity-detected', {
          detail: {
            provider: pk.provider,
            channelKey,
            // A YouTube membership milestone keeps the `membership` msg-id on the
            // chat row (the only id the message renderer decorates) and is told
            // apart here by the tag, so the feed can label it as a milestone.
            msgId:
              eventMsgType === 'membership' && tags['msg-param-milestone']
                ? 'member_milestone'
                : eventMsgType,
            username: parsed.username,
            displayName: parsed.display_name || parsed.username,
            userId: parsed.user_id,
            color: parsed.color,
            months: cumulative ?? parsed.metadata?.months,
            streak: streak && streak > 0 ? streak : undefined,
            tier: tags['msg-param-sub-plan'],
            giftCount: giftCount ?? ttGiftCount,
            giftName: ttGiftName,
            giftImage: ttGiftImage,
            giftDiamonds: ttGiftDiamonds,
            likeCount: ttLikeCount,
            avatarUrl: actorAvatar,
            originId,
            badges,
            systemText: parsed.metadata?.system_message,
            amount: scAmount,
            currency: tags['sc-currency'] || undefined,
            message: tags['sc-message'] || undefined,
          },
        }),
      );
    }

    // Channel-point redemptions that posted to chat (Twitch only): a highlighted
    // message or a reward that required text. On channels you only watch these
    // are the ONLY visible redemptions (the rest need broadcaster auth), and
    // Twitch sends just the reward id (no name) so custom rewards stay generic.
    if (pk.provider === 'twitch') {
      const isHighlight = tags['msg-id'] === 'highlighted-message';
      if (isHighlight || tags['custom-reward-id']) {
        window.dispatchEvent(
          new CustomEvent('provider-activity-detected', {
            detail: {
              provider: 'twitch',
              channelKey,
              msgId: 'channelpoints',
              username: parsed.username,
              displayName: parsed.display_name || parsed.username,
              userId: parsed.user_id,
              color: parsed.color,
              systemText: isHighlight ? 'highlighted message' : undefined,
            },
          }),
        );
      }
    }
  }
}

function handleRawIrcString(raw: string) {
  // USERNOTICE → dispatch global subscription event (for the badge/sub tracker)
  if (raw.includes('USERNOTICE')) {
    const loginMatch = raw.match(/(?:^|;)login=([^;]+)/);
    const msgIdMatch = raw.match(/(?:^|;)msg-id=([^;]+)/);
    const displayNameMatch = raw.match(/(?:^|;)display-name=([^;]+)/);
    const login = loginMatch?.[1];
    const msgId = msgIdMatch?.[1];
    const displayName = displayNameMatch?.[1];
    const subTypes = [
      'sub',
      'resub',
      'subgift',
      'submysterygift',
      'giftpaidupgrade',
      'primepaidupgrade',
      'anongiftpaidupgrade',
    ];
    if (login && msgId && subTypes.includes(msgId)) {
      window.dispatchEvent(
        new CustomEvent('twitch-subscription-detected', {
          detail: { login: login.toLowerCase(), msgId, displayName, rawMessage: raw },
        }),
      );
    }
  }

  const channel = extractChannelFromIrc(raw);
  if (!channel) return;
  const slice = useChatConnectionStore.getState().channels.get(channel);
  if (!slice) return;

  const idMatch = raw.match(/(?:^|;)id=([^;]+)/);
  const messageId = idMatch?.[1];
  const userIdMatch = raw.match(/(?:^@|;)user-id=([^;]+)/);
  const userId = userIdMatch?.[1];

  // Deterministic own-message upgrade (Helix-stamped real id awaiting its echo):
  // replace the stamped optimistic string in place with the full server line.
  if (messageId && slice.pendingUpgradeIds.has(messageId) && !slice.seenMessageIds.has(messageId)) {
    slice.pendingUpgradeIds.delete(messageId);
    const idMatchIdx = slice.messages.findIndex((m) =>
      typeof m === 'string'
        ? m.match(/(?:^@|;)id=([^;]+)/)?.[1] === messageId
        : isOwnOptimistic(m) && m.id === messageId,
    );
    if (idMatchIdx !== -1) {
      replaceMessageAt(slice, idMatchIdx, raw);
      slice.seenMessageIds.add(messageId);
      scheduleFlush();
      return;
    }
  }

  if (userId && isOwnUserId(userId)) {
    // Badge cache tracks only the PRIMARY (the IRC-connected account).
    if (userId === currentUserId) {
      const badgesMatch = raw.match(/(?:^|;)badges=([^;]*)/);
      if (badgesMatch && badgesMatch[1]) {
        slice.userBadgesFromIrc = badgesMatch[1];
      }
    }
    const contentMatch = raw.match(/PRIVMSG #\w+ :(.+)$/);
    // Whitespace-tolerant, same reason as the structured path above.
    const serverContent = contentMatch?.[1];
    if (serverContent) {
      const withoutMention = serverContent.replace(/^@\S+\s+/, '');
      const optimisticIdx = slice.messages.findIndex(
        (m) =>
          isUnstampedOwn(m) &&
          (sameSentContent(m.content, serverContent) || sameSentContent(m.content, withoutMention)),
      );
      if (optimisticIdx !== -1) {
        replaceMessageAt(slice, optimisticIdx, raw);
        if (messageId) slice.seenMessageIds.add(messageId);
        scheduleFlush();
        return;
      }
    }
    // Primary with no matching optimistic: queue once here (prior behavior, e.g.
    // a message the user sent from another device). Secondaries fall through to
    // the generic id-deduped queue below so they still appear exactly once.
    if (userId === currentUserId) {
      if (messageId) slice.seenMessageIds.add(messageId);
      queueMessage(slice.channel, raw);
      return;
    }
  }

  // Raw-string rows are the parse-failure fallback only; the structured
  // path carries every filtered message, and Rust already decided it.

  if (messageId) {
    if (slice.seenMessageIds.has(messageId)) return;
    slice.seenMessageIds.add(messageId);
    trimSeenIds(slice);
    queueMessage(slice.channel, raw);
  } else {
    queueMessage(slice.channel, raw);
  }
}

// --- Public API -------------------------------------------------------------

/** Acquire a chat connection for `channel`. Idempotent — if the channel is
 *  already acquired, just increments the ref count. */
export async function acquireChannel(
  channel: string,
  channelId: string | null,
  provider: ProviderId = 'twitch',
): Promise<void> {
  // Twitch keeps bare-login keys (byte-identical to before); non-Twitch sources
  // get a "provider:channel" composite key. MultiChat only.
  // Folded to the STORED form. This used to build the case-preserving composite
  // and then miss its own slice for every mixed-case YouTube id, so the ref count
  // never rose and each acquire silently replaced the previous slice.
  const key = sliceLookupKey(provider, channel);
  const state = useChatConnectionStore.getState();
  const existing = state.channels.get(key);

  if (existing) {
    existing.refCount += 1;
    if (channelId && !existing.channelId) {
      existing.channelId = channelId;
      // MultiChat opens panes before the channel's broadcaster_id has resolved
      // (the stream-info poll runs after first render). That means the initial
      // acquireChannel call comes through with channelId=null, preloadChannel
      // bails, and the badge metadata cache never gets populated for this
      // channel. When the real channelId arrives a moment later and the caller
      // re-acquires to refresh it, kick off the badge init we deferred so
      // Twitch channel badges (subscriber/bits/etc.) start resolving.
      if (provider === 'twitch') void initializeBadgesForChannel(channelId);
    }
    bumpRevision();
    Logger.debug(`[ChatStore] +1 ref on ${key} (now ${existing.refCount})`);
    return;
  }

  const slice = emptySlice(key, channelId, provider);
  slice.refCount = 1;
  setSlice(key, slice);

  // Twitch: start the Rust backfill now, alongside the connect/join below,
  // and hold the first live rows until it lands (or HISTORY_HOLD_MS) so the
  // pane paints once with history above the live tail. Without an id there
  // is no preload (ensureChannelHistory runs it once the id resolves).
  if (provider === 'twitch' && channelId) {
    armHistoryHold(slice);
    startChannelHistory(key);
  }

  // Kick's socket carries only NEW traffic, so a freshly opened pane is empty
  // until somebody talks, and an OFFLINE channel stays empty indefinitely. Seed
  // it from Kick's own scrollback the way the site does. Fire-and-forget: the
  // socket connect below must not wait on it, and no scrollback is a cosmetic
  // loss, never a reason to fail opening the channel.
  if (provider === 'kick') {
    void seedKickHistory(key, channel);
  }

  // Arm the watchdog as soon as a channel exists, not only after a successful
  // connect. It used to start only inside connectBridgeForFirstChannel, so a
  // user whose FIRST connect failed had no safety net at all. Idempotent: it
  // clears any existing timer first.
  startHealthCheck();

  // First channel ever: open the bridge + WS. A failure here deliberately LEAVES
  // the slice in place: the watchdog armed just above is the recovery path, and
  // removing the slice would take channels.size to 0, which every recovery path
  // (watchdog, visibilitychange, online) treats as "nothing to do". Consumers
  // that abandon a failed acquire release the channel on unmount.
  if (state.channels.size === 0) {
    await connectBridgeForFirstChannel(key, channelId, false, provider, channel);
  } else if (provider === 'twitch') {
    // Bridge already up. If a Twitch IRC connection is already running (another
    // Twitch slice exists), JOIN onto it. If not - the bridge was opened by a
    // non-Twitch provider first - START the Twitch IRC on the shared bridge,
    // since join_chat_channel would have nothing to join. The WS is already open
    // so it isn't reopened. (In an all-Twitch session another Twitch slice always
    // exists here, so this stays join_chat_channel: byte-identical to before.)
    const hasOtherTwitch = Array.from(
      useChatConnectionStore.getState().channels.values(),
    ).some((s) => s.channel !== key && s.provider === 'twitch');
    try {
      await invoke(hasOtherTwitch ? 'join_chat_channel' : 'start_chat', { channel: key });
      slice.isConnected = true;
      bumpRevision();
      void preloadChannel(key, channelId);
    } catch (err) {
      channelConnectFailed(key, 'twitch', key, err);
    }
  } else {
    // Non-Twitch source on the already-open bridge: connect its adapter (no
    // Twitch badge/history preload). The shared local-WS delivers its frames
    // once the adapter publishes, routed by the composite channel key.
    try {
      await invoke('provider_chat_connect', { provider, channel });
      slice.isConnected = true;
      bumpRevision();
    } catch (err) {
      channelConnectFailed(key, provider, channel, err);
    }
  }
}

/** Release a chat connection for `channel`. When the last consumer releases,
 *  the channel is PARTed; when the last channel is released, the WebSocket
 *  and Rust IRC service are torn down. */
export async function releaseChannel(
  channel: string,
  provider: ProviderId = 'twitch',
): Promise<void> {
  // Same fold as acquire. Before this, a mixed-case YouTube id missed here and hit
  // the early return below, so the channel was never PARTed and its slice never
  // freed: the connection outlived the tile and kept delivering into a grid that
  // no longer had a slice for it.
  const key = sliceLookupKey(provider, channel);
  const slice = useChatConnectionStore.getState().channels.get(key);
  if (!slice) return;
  slice.refCount -= 1;
  Logger.debug(`[ChatStore] -1 ref on ${key} (now ${slice.refCount})`);
  if (slice.refCount > 0) {
    bumpRevision();
    return;
  }
  // Last consumer for this channel — drop the slice and PART the channel on
  // the IRC side so messages stop flowing in for it. Critically, we do NOT
  // call `stop_chat` here even when this window's channel set goes empty —
  // the Rust IRC connection is process-wide and other windows (the main app,
  // sibling MultiChat popouts) may still be using it. Tearing down here
  // would kill chat for every other consumer in the process.
  removeSlice(key);
  clearChannelRetry(key);
  // Free per-channel state the slice didn't own: the pending flush queue and the
  // resolved emote set (1 to 3 MB of metadata that otherwise stayed pinned for
  // the whole session after the last consumer left). emoteSubscribers is left to
  // its component-driven unsubscribe lifecycle.
  pendingByChannel.delete(key);
  emoteCache.delete(key);
  // Open repeat runs point at message ids in the buffer we just dropped.
  resetRepeatRuns(key);
  inflightEmoteFetches.delete(key);
  try {
    if (provider === 'twitch') {
      await invoke('leave_chat_channel', { channel: key });
    } else {
      await invoke('provider_chat_disconnect', { provider, channel });
    }
  } catch (err) {
    Logger.warn(`[ChatStore] leave failed for ${key}:`, err);
  }

  // If this window's local channel list is now empty, tear down only this
  // window's local WebSocket connection — the Rust IRC service keeps running
  // for other consumers. A subsequent acquireChannel from this window will
  // re-open its local socket via start_chat (which is now idempotent and
  // returns the existing port without disrupting other windows).
  const remaining = useChatConnectionStore.getState().channels.size;
  if (remaining === 0) {
    intentionalDisconnect = true;
    // Invalidate any connect still in flight so it discards its socket instead
    // of undoing this teardown when it finally resolves.
    connectGeneration += 1;
    // Drop any queued reconnect intent with it, or a request that arrived
    // mid-teardown re-arms a timer 500ms after this deliberately cleared one.
    reconnectPending = false;
    reconnectForcePending = false;
    backendReconnecting = false;
    clearHealthCheck();
    clearAllChannelRetries();
    clearPendingLostRow();
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (ws) {
      ws.onmessage = null;
      ws.onerror = null;
      ws.onclose = null;
      ws.onopen = null;
      try {
        ws.close(1000, 'Last channel released');
      } catch {
        // ignore
      }
      ws = null;
    }
    intentionalDisconnect = false;
    useChatConnectionStore.setState({ wsPort: null });
  }
}

/** The row being replied to, as the fields a reply carries. Falls back to the
 *  parent's id alone when the row has scrolled out of this window. */
function replyParentFor(slice: ChannelSlice, parentId: string) {
  const parent = slice.messages.find((m) =>
    typeof m === 'string' ? m.match(/(?:^@|;)id=([^;]+)/)?.[1] === parentId : (m as any)?.id === parentId,
  );
  if (!parent) return { id: parentId };
  if (typeof parent === 'string') {
    return {
      id: parentId,
      display_name: parent.match(/(?:^@|;)display-name=([^;]*)/)?.[1] ?? '',
      login: parent.match(/:(\w+)!\w+@\w+\.tmi\.twitch\.tv PRIVMSG/)?.[1] ?? '',
      user_id: parent.match(/(?:^@|;)user-id=([^;]+)/)?.[1] ?? '',
      body: parent.match(/PRIVMSG #\w+ :(.+)$/)?.[1] ?? '',
    };
  }
  const p = parent as BackendChatMessage;
  return { id: parentId, display_name: p.display_name || '', login: p.username || '', user_id: p.user_id || '', body: p.content || '' };
}

/** Send a message to `channel`. The row shown for it is built by Rust with
 *  channel-correct room-id and badge metadata. */
export async function sendChannelMessage(
  channel: string,
  rawText: string,
  userInfo: SendUserInfo,
  replyParentMsgId?: string,
  senderAccount?: SendAsAccount | null,
): Promise<void> {
  // Trailing whitespace never survives the round trip: Twitch's echo comes
  // back without it (and the Rust parser trim_end()s the payload), while the
  // emote picker always leaves "name " in the compose box. An optimistic row
  // carrying that space failed the content match whenever the IRC echo beat
  // the Helix id stamp, so the echo was appended as a second copy of your own
  // message until the stamp landed and the duplicate id got collapsed: two
  // identical rows on screen for a beat, then one vanished. Send what Twitch
  // will echo. (The duplicate-bypass suffix ends in U+E0000, which is not
  // whitespace, so it is untouched.)
  const text = rawText.trimEnd();
  if (!text.trim()) return;
  const key = channel.toLowerCase();
  const slice = useChatConnectionStore.getState().channels.get(key);
  if (!slice || !slice.isConnected) return;

  // currentUserId tracks the PRIMARY (the IRC-connected reader), regardless of
  // which account we're sending as.
  currentUserId = userInfo.userId;

  // Who the message is sent AS. Defaults to the primary; a chosen secondary
  // sends with its own identity + token (resolved in the backend by the
  // senderAccountId below). The secondary is registered as "own" so its echo
  // reconciles against the optimistic copy rather than duplicating.
  const sendingAsSecondary = !!senderAccount && senderAccount.userId !== userInfo.userId;
  const senderUserId = senderAccount?.userId ?? userInfo.userId;
  const senderUsername = sendingAsSecondary ? senderAccount!.login : userInfo.username;
  const senderDisplayName = sendingAsSecondary ? senderAccount!.displayName : userInfo.displayName;
  if (sendingAsSecondary) {
    ownAccountIds.add(senderUserId);
  }

  const tempId = `local-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
  // For the primary, prefer the real USERSTATE color (live, refreshes on /color)
  // over the build-time default so the username doesn't flash a wrong color until
  // USERSTATE repaints it. A secondary carries its own resolved color.
  const color =
    (sendingAsSecondary
      ? senderAccount!.color
      : userInfo.color || slice.userColorFromIrc) || '#9147FF';
  // USERSTATE-cached badges win because they're tenure-correct for this channel;
  // caller-provided badges are the fallback while USERSTATE hasn't landed. A
  // secondary isn't the IRC-connected user, so it has no cached badges; its echo
  // repaints them via the id-match path.
  const badges = sendingAsSecondary ? '' : slice.userBadgesFromIrc || userInfo.badges || '';

  // The row is built in Rust by the same parser as received messages (segments,
  // emotes, reply info), so it renders exactly as the rest of chat. Requested
  // alongside the send rather than before it: the send must not wait on it.
  const built = invoke<BackendChatMessage | null>('build_own_chat_message', {
    message: {
      channel: key,
      text,
      local_id: tempId,
      sender_id: senderUserId,
      sender_login: senderUsername,
      sender_display_name: senderDisplayName,
      color,
      badges,
      room_id: slice.channelId ?? '',
      reply_to: replyParentMsgId ? replyParentFor(slice, replyParentMsgId) : null,
    },
  }).catch((err) => {
    Logger.warn('[ChatStore] could not build the sent row:', err);
    return null;
  });
  const sending = invoke<{
      message_id: string | null;
      is_sent: boolean;
      drop_reason: string | null;
    }>('send_chat_message', {
      message: text,
      replyParentMsgId: replyParentMsgId || null,
      targetChannel: key,
      broadcasterId: slice.channelId || null,
      senderId: senderUserId || null,
      senderAccountId: sendingAsSecondary ? senderUserId : null,
    });
  // Awaited below; this only stops a fast failure reporting as unhandled while
  // the row is still being built.
  sending.catch(() => {});

  const row = await built;
  if (row) {
    slice.seenMessageIds.add(tempId);
    pushMessage(slice, { ...row, own_optimistic: true });
    // The CHANNEL's counter, not just the global one: a pane re-renders on its
    // own channel's revision, and Twitch never echoes your own message back, so
    // a global-only bump left your message invisible until someone else spoke.
    bumpRevisionFor([key]);
  }

  try {
    const result = await sending;

    // Twitch accepted the request but dropped the message (AutoMod, etc.).
    // Pull the optimistic copy and tell the user why.
    if (result && result.is_sent === false) {
      slice.messages = slice.messages.filter((m) => {
        if (typeof m === 'string') return !m.includes(`id=${tempId}`);
        return (m as any)?.id !== tempId;
      });
      slice.seenMessageIds.delete(tempId);
      bumpRevisionFor([key]);
      if (result.drop_reason) {
        injectSystemMessage(key, `Your message was not sent: ${result.drop_reason}`);
      }
      return;
    }

    // Helix returns the authoritative message id. Stamp it onto the optimistic
    // copy so deletes work immediately, with no dependency on catching the IRC
    // echo. We do NOT mark the real id as "seen": when the echo arrives it
    // upgrades this copy in place (real badges/tenure) via the id-match path in
    // appendStructuredMessage / handleRawIrcString.
    if (result && result.message_id) {
      const realId = result.message_id;
      const idx = slice.messages.findIndex((m) => isOwnOptimistic(m) && m.id === tempId);
      if (idx !== -1) {
        replaceMessageAt(slice, idx, { ...(slice.messages[idx] as BackendChatMessage), id: realId });
        slice.seenMessageIds.delete(tempId);
        // Arm the echo-upgrade fast path for this id. Defensive cap: a stamped
        // row whose echo never arrives costs one stale entry, never growth.
        slice.pendingUpgradeIds.add(realId);
        if (slice.pendingUpgradeIds.size > 32) {
          const oldest = slice.pendingUpgradeIds.values().next().value;
          if (oldest !== undefined) slice.pendingUpgradeIds.delete(oldest);
        }
        bumpRevisionFor([key]);
      }
    }
  } catch (err) {
    Logger.error('[ChatStore] send_chat_message failed:', err);
    slice.messages = slice.messages.filter((m) => {
      if (typeof m === 'string') return !m.includes(`id=${tempId}`);
      return (m as any)?.id !== tempId;
    });
    slice.seenMessageIds.delete(tempId);
    bumpRevisionFor([key]);
    throw err;
  }
}

/** Inject a system message into the channel (mirrors the `twitch-system-message`
 *  custom event the old hook listened for). Used by `/mods` and similar local
 *  command results. */
/** The platform label/color for a system row on this channel key, so a notice on
 *  a Kick or YouTube channel isn't attributed to Twitch. */
export function systemSourceFor(channelKey: string): { label: string; color: string } | undefined {
  const provider = parseKey(channelKey).provider;
  return provider === 'twitch' ? undefined : { label: PROVIDERS[provider].label, color: PROVIDERS[provider].color };
}

export function injectSystemMessage(
  channel: string,
  message: string,
  songCard?: SongMatch,
  // Which platform is speaking. Defaults to Twitch so every existing caller is
  // unchanged; a provider pane passes its own so the row doesn't sign itself
  // "Twitch" on a Kick or YouTube channel.
  source?: { label: string; color: string },
): void {
  const sysMsgId = `sys-cmd-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
  withSlice(channel, (slice) => {
    pushMessage(slice, {
      id: sysMsgId,
      // Stamped so chronological backfill insertion can order gap messages
      // around system rows (e.g. the disconnect marker) instead of past them.
      timestamp: String(Date.now()),
      username: 'System',
      display_name: source?.label ?? 'Twitch',
      color: source?.color ?? '#9147ff',
      badges: [{ key: 'staff/1', info: {} }],
      content: message,
      segments: [{ type: 'text', content: message }],
      is_action: false,
      is_first_message: false,
      is_mentioned: false,
      is_from_shared_chat: false,
      songCard,
      tags: new Map([
        ['user-id', 'tw-system'],
        ['id', sysMsgId],
      ]),
    });
    slice.seenMessageIds.add(sysMsgId);
  });
}

/** Inject a no-input channel-points redemption as a chat row. Reuses the native
 *  highlight-message render path (via the `custom-reward-id` tag) so it reads as
 *  a redemption, with the redeemer as the author and the reward name as the body.
 *  No-ops when the channel's chat isn't open. Message-style rewards post their
 *  own PRIVMSG, so callers should only pass the no-input ones. */
export function injectRedemptionMessage(
  channel: string,
  r: {
    userLogin: string;
    userName: string;
    userId?: string;
    rewardId: string;
    rewardTitle: string;
    cost?: number;
    color?: string;
    redemptionId?: string;
    pointsIconUrl?: string | null;
    rewardImageUrl?: string;
    rewardBackground?: string;
  },
): void {
  // A stable id from Twitch's redemption id (when present) makes this idempotent:
  // the same redemption seen by two open chat views collapses to one row.
  const id = r.redemptionId
    ? `redeem-${r.redemptionId}`
    : `redeem-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
  const login = r.userLogin || r.userName;
  const name = r.userName || r.userLogin || login;
  // Body is just the reward name; the cost renders as the channel-points glyph +
  // amount in ChatMessage (via the sn-reward-cost / sn-points-icon tags), not as
  // a plain "(N)" appended to the text.
  const body = r.rewardTitle;
  // Plain-object tags (NOT a Map): parseMessage rebuilds tags with
  // `Object.entries(raw.tags)`, which is empty for a Map — that silently dropped
  // custom-reward-id, so redemptions lost their decoration and rendered plain.
  const tags: Record<string, string> = {
    'user-id': r.userId || '',
    id,
    'display-name': name,
    // Triggers the redemption highlight + label in ChatMessage.
    'custom-reward-id': r.rewardId || 'sn-redemption',
    // Marks the row as a redemption for the stream overlay renderer, which draws
    // it as a Channel points event (the hosted overlay builds the same tags).
    'sn-reward-title': r.rewardTitle,
  };
  if (r.rewardImageUrl) tags['sn-reward-image'] = r.rewardImageUrl;
  if (r.rewardBackground) tags['sn-reward-bg'] = r.rewardBackground;
  if (r.cost && r.cost > 0) tags['sn-reward-cost'] = String(r.cost);
  if (r.pointsIconUrl) tags['sn-points-icon'] = r.pointsIconUrl;
  withSlice(channel, (slice) => {
    if (slice.seenMessageIds.has(id)) return;
    pushMessage(slice, {
      id,
      username: login,
      display_name: name,
      color: r.color || '#9147ff',
      badges: [],
      content: body,
      segments: tokenizeLocalBody(body, getChannelEmotes(channel)),
      is_action: false,
      is_first_message: false,
      is_mentioned: false,
      is_from_shared_chat: false,
      user_id: r.userId || '',
      tags,
    });
    slice.seenMessageIds.add(id);
  });
}

export function setChannelPaused(channel: string, paused: boolean): void {
  withSlice(channel, (slice) => {
    if (!paused && slice.isPausedForBuffer) {
      // Record how far above the cap the paused buffer got; flushPending
      // releases it gradually from there.
      slice.resumeOverflow = resumeOverflowFor(slice.messages.length, getActiveHistoryMax());
    }
    slice.isPausedForBuffer = paused;
    // No trim here: cutting the paused overflow to historyMax in one step
    // deleted up to CHAT_BUFFER_SIZE rows the user had scrolled up to read,
    // the moment they resumed. flushPending decays the overflow gradually
    // (RESUME_DECAY_PER_FLUSH rows per flush, from the top) instead.
  });
}

// --- React hooks ------------------------------------------------------------

/** Snapshot shape consumed by ChatWidget / MultiChat tabs. */
export interface ChannelChatSnapshot {
  messages: any[];
  isConnected: boolean;
  error: string | null;
  roomState: RoomState;
  userBadges: string | null;
  deletedMessageIds: Set<string>;
  clearedUserContexts: Map<string, ClearedUserEntry>;
  /** Monotonic count of live messages received (see ChannelSlice). */
  liveMessageCount: number;
  /** Currently pinned message (provider-driven, e.g. Kick's pin event), or null.
   *  Shaped like ChatWidget's PinnedMessage so it can feed the same banner. */
  pinnedMessage: any | null;
  /**
   * Changes whenever anything about this channel's chat changed. Pass it to the
   * memoized message list as its re-render trigger.
   *
   * Since the copy-on-write change in the store, `messages`,
   * `deletedMessageIds` and `clearedUserContexts` also change identity with
   * their content (flushPending, pushMessage, the own-echo upgrades, the
   * repaints and the moderation marks all write a fresh container), so array
   * identity is a truthful signal again. The token is kept as the second,
   * channel-wide signal: it also covers changes to the fields above that are
   * not part of the list's props. Keep passing it.
   */
  renderToken: number;
}

const EMPTY_SNAPSHOT: ChannelChatSnapshot = {
  messages: [],
  isConnected: false,
  error: null,
  roomState: { ...EMPTY_ROOM_STATE },
  userBadges: null,
  deletedMessageIds: new Set(),
  clearedUserContexts: new Map(),
  liveMessageCount: 0,
  pinnedMessage: null,
  renderToken: 0,
};

/** React hook returning the live message count for a channel. */
export function useChannelMessageCount(channel: string | null | undefined): number {
  const key = channel ? channel.toLowerCase() : null;
  useChatConnectionStore((state) => (key ? state.revisionByChannel[key] ?? 0 : state.revision));
  if (!channel) return 0;
  const slice = useChatConnectionStore.getState().channels.get(channel.toLowerCase());
  return slice ? slice.messages.length : 0;
}

/** True when this message mentions `login` (case-insensitive). Handles both
 *  the parsed-object form (Rust ChatMessage with optional `is_mentioned` set
 *  by the segment parser) and the raw IRC-string fallback (regex-scan the
 *  PRIVMSG body for `@login`). Used by the unread-mention counter, which only
 *  surfaces unread badges for @ mentions of the signed-in user. */
function messageMentionsLogin(msg: unknown, login: string): boolean {
  if (!msg || !login) return false;
  if (typeof msg === 'object') {
    const obj = msg as { is_mentioned?: boolean; content?: string };
    if (obj.is_mentioned) return true;
    if (typeof obj.content === 'string') {
      return obj.content.toLowerCase().includes(`@${login}`);
    }
    return false;
  }
  if (typeof msg === 'string') {
    const idx = msg.indexOf(' PRIVMSG ');
    if (idx === -1) return false;
    const colon = msg.indexOf(' :', idx);
    if (colon === -1) return false;
    return msg.slice(colon + 2).toLowerCase().includes(`@${login}`);
  }
  return false;
}

/** React hook returning the count of messages mentioning the supplied login
 *  in a channel. Used by the MultiChat popout's tab strip to drive @-mention
 *  unread indicators — comparing this count against a per-tab "last seen"
 *  snapshot reveals new mentions that arrived while the tab wasn't visible.
 *  Pass `null` for `login` (e.g. unauthenticated) and the count stays at 0. */
export function useChannelMentionCount(
  channel: string | null | undefined,
  login: string | null | undefined,
): number {
  const key = channel ? channel.toLowerCase() : null;
  useChatConnectionStore((state) => (key ? state.revisionByChannel[key] ?? 0 : state.revision));
  if (!channel || !login) return 0;
  const slice = useChatConnectionStore.getState().channels.get(channel.toLowerCase());
  if (!slice) return 0;
  const target = login.toLowerCase();
  let count = 0;
  for (const msg of slice.messages) {
    if (messageMentionsLogin(msg, target)) count++;
  }
  return count;
}

/** React hook returning the shared per-channel emote set. Multiple components
 *  consuming the same channel share one EmoteSet reference (no duplication
 *  across split panes). Returns null until the fetch lands. */
export function useChannelEmotes(
  channel: string | null | undefined,
  channelId: string | null | undefined,
  provider: ProviderId = 'twitch',
): EmoteSet | null {
  const key = channel ? emoteCacheKey(channel, provider) : null;
  const [version, setVersion] = useState(0);

  useEffect(() => {
    if (!key || !channel) return;
    const unsubscribe = subscribeChannelEmotes(key, () => setVersion((v) => v + 1));
    // Kick fetches by slug (no channelId needed); Twitch needs the numeric id.
    if (provider === 'kick' || channelId) void ensureChannelEmotes(channel, channelId ?? '', provider);
    return unsubscribe;
  }, [key, channel, channelId, provider]);

  // Re-read on each version bump
  void version;
  return key ? emoteCache.get(key) ?? null : null;
}

/** React hook returning the per-channel snapshot. Pass `null` while no channel
 *  is acquired — the hook returns an empty snapshot in that case so callers
 *  don't need to null-guard the entire return object. */
/** Low-frequency slice fields for the widget chrome (header, composer,
 *  room-state chips). Shallow-compared, so per-message flushes evaluate the
 *  selector (cheap: one Map.get) but re-render the subscriber only when one of
 *  these fields actually changed. Every writer of these fields already lands a
 *  setState (withSlice / bumpRevision / setSlice), so no extra signal is
 *  needed and no writer can be missed. */
export function useChannelChatMeta(channel: string | null | undefined) {
  const key = channel ? channel.toLowerCase() : null;
  return useChatConnectionStore(
    useShallow((state) => {
      const slice = key ? state.channels.get(key) : undefined;
      return {
        isConnected: slice?.isConnected ?? false,
        error: slice?.error ?? null,
        roomState: slice?.roomState ?? EMPTY_ROOM_STATE,
        userBadges: slice?.userBadges ?? null,
        pinnedMessage: slice?.pinnedMessage ?? null,
      };
    }),
  );
}

export function useChannelChat(channel: string | null | undefined): ChannelChatSnapshot {
  const key = channel ? channel.toLowerCase() : null;
  // Subscribe to revision to drive updates; read the slice imperatively to
  // avoid Map.get returning new references on every render. The revision is
  // also handed back as `renderToken` (see ChannelChatSnapshot) so memoized
  // consumers have a change signal that in-place message mutations can't hide.
  // No key means the caller is showing something this store does not own (a VOD
  // replay, or a merged feed that subscribes to its own sources). Falling back to
  // the GLOBAL revision there subscribed it to every channel in the app — every
  // MultiNook tile and MultiChat pane — so an unrelated flush re-rendered the
  // message list. That is the one hot path the incremental-merge work exists to
  // keep quiet. A constant subscribes to nothing.
  const renderToken = useChatConnectionStore((state) =>
    key ? state.revisionByChannel[key] ?? 0 : 0,
  );
  if (!key) return EMPTY_SNAPSHOT;
  const slice = useChatConnectionStore.getState().channels.get(key);
  if (!slice) return EMPTY_SNAPSHOT;
  return {
    messages: slice.messages,
    isConnected: slice.isConnected,
    error: slice.error,
    roomState: slice.roomState,
    userBadges: slice.userBadges,
    deletedMessageIds: slice.deletedMessageIds,
    clearedUserContexts: slice.clearedUserContexts,
    liveMessageCount: slice.liveMessageCount,
    pinnedMessage: slice.pinnedMessage,
    renderToken,
  };
}

// Listen for visibility regain to nudge a reconnect if the WS died while hidden
if (typeof document !== 'undefined') {
  onWindowVisibility(() => {
    if (isWindowHidden()) return;
    // Only a healthy socket is a reason to do nothing. The old `!ws ||` bailed
    // when the socket was GONE, which is precisely the state this handler was
    // written to rescue.
    if (ws && ws.readyState === WebSocket.OPEN) return;
    if (useChatConnectionStore.getState().channels.size === 0) return;
    Logger.debug('[ChatStore] Visibility regained, scheduling reconnect');
    scheduleReconnect(0);
  });
}

// Network came back: the flaky-connection case this whole ladder exists for.
// `online` tracks the adapter rather than real reachability, so this can fire
// while the internet is still down; harmless, since the reconnect it schedules
// has its own backoff.
if (typeof window !== 'undefined') {
  window.addEventListener('online', () => {
    if (ws && ws.readyState === WebSocket.OPEN) return;
    if (useChatConnectionStore.getState().channels.size === 0) return;
    Logger.warn('[ChatStore] Network regained, scheduling reconnect');
    scheduleReconnect(0);
  });
}

// Listen for locally emitted system messages (matches the prior hook contract)
if (typeof window !== 'undefined') {
  window.addEventListener('twitch-system-message', ((e: CustomEvent) => {
    const message = e.detail?.message;
    if (!message) return;
    const songCard = e.detail?.songCard as SongMatch | undefined;
    const channels = useChatConnectionStore.getState().channels;
    if (channels.size === 0) return;
    // System messages from `/mods` etc. apply to the channel that issued the
    // command; today only one channel is acquired by the main app, so we
    // route to the single acquired channel. Multi-channel callers should use
    // `injectSystemMessage(channel, message)` directly.
    if (channels.size === 1) {
      const ch = channels.keys().next().value as string;
      injectSystemMessage(ch, message, songCard);
    }
  }) as EventListener);
}

// --- Dev-only chat flood injector -------------------------------------------
//
// window.__snChatFlood(channel, perSecond = 400, seconds = 10) pushes synthetic
// structured messages through the real ingestion path (queueMessage ->
// flushPending), so the rAF coalescer, the buffer policy and the message list
// are exercised exactly as a live burst would exercise them: no second
// coalescer, no bypass. Each message is a clone of the newest structured
// message in the slice with a fresh id, content and timestamp, so badges and
// the row layout render realistically. Resolves with { sent } when the burst
// ends. Stripped from production builds by the DEV guard.
if (import.meta.env.DEV && typeof window !== 'undefined') {
  const FLOOD_LINES = [
    'oh wow, groundbreaking gameplay, truly never seen anything like it before',
    'that was a diff so hard it should honestly be studied, throw of the century',
    '7TV genuinely carries this entire chat, the emotes are elite',
    'best stream on twitch no cap, the chat never misses',
    'LOL',
    'W',
  ];
  (window as unknown as Record<string, unknown>).__snChatFlood = (
    channel: string,
    perSecond = 400,
    seconds = 10,
  ): Promise<{ sent: number }> => {
    const key = channel.toLowerCase();
    const slice = useChatConnectionStore.getState().channels.get(key);
    if (!slice) return Promise.reject(new Error(`no slice for ${key}`));
    const template = [...slice.messages]
      .reverse()
      .find((m) => typeof m === 'object' && m !== null && Array.isArray((m as { segments?: unknown }).segments));
    if (!template) return Promise.reject(new Error('no structured message to clone'));
    const total = Math.max(1, Math.round(perSecond * seconds));
    const tickMs = 20;
    const perTick = Math.max(1, Math.round(perSecond * tickMs / 1000));
    let sent = 0;
    return new Promise((resolve) => {
      const timer = setInterval(() => {
        for (let i = 0; i < perTick && sent < total; i++) {
          const text = FLOOD_LINES[sent % FLOOD_LINES.length];
          const now = Date.now();
          const id = `flood-${now}-${sent}`;
          const t = template as Record<string, unknown>;
          queueMessage(key, {
            ...t,
            id,
            content: text,
            segments: [{ type: 'text', content: text }],
            timestamp: String(now),
            tags: { ...(t.tags as Record<string, unknown>), id, 'tmi-sent-ts': String(now) },
          });
          sent++;
        }
        if (sent >= total) {
          clearInterval(timer);
          resolve({ sent });
        }
      }, tickMs);
    });
  };
}
