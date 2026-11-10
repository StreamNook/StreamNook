// The stream manager's Rust view and door, typed for the desktop and Android
// shells. Rust decides who may do what, words every line and runs every write
// (services/stream_manager.rs); these types only mirror what it sends.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { useAccountRosterStore, type AccountRoster } from '../stores/accountRosterStore';
import { isBackgrounded } from '../mobile/backgroundGate';
import { forgetAccount } from '../components/chat/managerStore';

export type ManagerRole = 'broadcaster' | 'editor' | 'moderator' | 'moderator_editor' | 'viewer';
export type ManagerSection = 'info' | 'poll' | 'prediction' | 'raid' | 'ads' | 'rewards' | 'chat' | 'announce';

/** A page of the Manage channel window. */
export type PageId =
  | 'info'
  | 'poll'
  | 'prediction'
  | 'raid'
  | 'ads'
  | 'rewards'
  | 'announce'
  | 'chat_modes'
  | 'moderation';

/** One row of the window's side list, in Rust's order, only what the role
 *  may use here. */
export interface PageRow {
  id: PageId;
  /** "Your channel", "Channel" or "Chat": the heading it sits under. */
  group: string;
  label: string;
  /** "running", "locked", "counting down": a live mark, in words. */
  live: string | null;
  /** A short count ("3"). */
  badge: string | null;
}

export interface Choice {
  label: string;
  value: number;
}

export interface SectionRow {
  id: ManagerSection;
  label: string;
  detail: string | null;
  badge: string | null;
}

export interface Category {
  id: string;
  name: string;
  box_art_url: string;
}

export interface StreamInfoView {
  title: string;
  category: Category | null;
  tags: string[];
  language: string;
  language_name: string;
  languages: { code: string; name: string }[];
  summary: string;
  /** The category in use first, then the ones saved here before. */
  recent_categories: Category[];
  /** The row the title's emoji menu opens on, and its heading. */
  title_emoji: string[];
  title_emoji_heading: string;
}

export interface RunningPoll {
  id: string;
  title: string;
  /** When it ends, on this device's clock (Rust turns Twitch's time local). */
  ends_at_ms: number | null;
}

export interface RunningPrediction {
  id: string;
  title: string;
  status: 'active' | 'locked';
  outcomes: { id: string; title: string; color: string }[];
  /** When it stops taking predictions, on this device's clock; only while active. */
  locks_at_ms: number | null;
}

/** A poll or prediction started here before, for the builder's Recent list. */
export interface RecentPoll {
  title: string;
  options: string[];
}

export interface RaidView {
  target_id: string;
  target_login: string;
  target_name: string;
  deadline_ms: number;
  /** The whole countdown, for the ring. */
  total_ms: number;
}

export interface AdSchedule {
  next_ad_ms: number | null;
  next_ad_seconds: number;
  preroll_free_seconds: number;
  snoozes_left: number;
  snooze_refresh_ms: number | null;
  snooze_line: string;
  preroll_line: string | null;
}

export interface AdsView {
  lengths: Choice[];
  schedule: AdSchedule | null;
  schedule_hint: string | null;
}

export interface ChatModeRow {
  kind: 'followers' | 'slow' | 'subs' | 'emotes' | 'unique';
  label: string;
  on: boolean;
  value: number | null;
  value_label: string | null;
  options: Choice[];
}

export interface ChatView {
  modes: ChatModeRow[];
  shield: boolean | null;
  announce_colors: { id: string; label: string }[];
}

export interface FormLimits {
  title: number;
  tags: number;
  tag: number;
  poll_title: number;
  poll_choice: number;
  poll_choices_min: number;
  poll_choices_max: number;
  poll_seconds_min: number;
  poll_seconds_max: number;
  prediction_title: number;
  prediction_outcome: number;
  prediction_outcomes_min: number;
  prediction_outcomes_max: number;
  prediction_seconds_min: number;
  prediction_seconds_max: number;
  durations: Choice[];
}

export interface StreamManagerView {
  channel_id: string;
  channel_login: string;
  role: ManagerRole;
  role_line: string;
  sections: SectionRow[];
  /** The Manage channel window's side list. */
  pages: PageRow[];
  info: StreamInfoView | null;
  poll: RunningPoll | null;
  prediction: RunningPrediction | null;
  raid: RaidView | null;
  ads: AdsView | null;
  chat: ChatView | null;
  queue_count: number | null;
  limits: FormLimits;
  problems: string[];
  /** Rust's read counter: a view with a lower one is older and never wins. */
  rev: number;
  /** The Twitch account this view was read as. */
  user_id: string;
  /** False when that part could not be read; Rust then sends the last one it knew. */
  poll_known: boolean;
  prediction_known: boolean;
  /** Newest first, at most 10, kept by Rust on this device. */
  recent_polls: RecentPoll[];
  recent_predictions: RecentPoll[];
}

export interface StreamInfoDraft {
  title: string;
  category: Category | null;
  tags: string[];
  language: string;
}

export interface InfoCheck {
  ok: boolean;
  problems: { field: string; line: string }[];
  title_count: string;
  /** How the count reads: quiet, near the limit, or past it. */
  title_tone: 'quiet' | 'near' | 'over';
  tags_count: string;
  preview: {
    title: string;
    channel_name: string;
    avatar_url: string | null;
    category_name: string;
    box_art_url: string | null;
    tags: string[];
    thumbnail_url: string | null;
    /** While live: the count and start the live card's chips show. */
    viewer_count?: number | null;
    started_at?: string | null;
    /** Twitch partner: the verified mark after the name. */
    partner?: boolean;
  };
  changed: boolean;
}

export interface QueueRow {
  id: string;
  user_login: string;
  user_name: string;
  avatar_url: string | null;
  input: string | null;
  at_ms: number;
  line: string;
}

export interface QueueGroup {
  reward_id: string;
  title: string;
  cost: number;
  paused: boolean;
  enabled: boolean;
  image_url: string | null;
  color: string | null;
  heading: string;
  rows: QueueRow[];
}

export interface QueueView {
  groups: QueueGroup[];
  count: number;
  empty_line: string | null;
  more: boolean;
  rev: number;
  user_id: string;
}

export interface RaidTarget {
  id: string;
  login: string;
  name: string;
  avatar_url: string | null;
  game_name: string;
  live: boolean;
  line: string;
}

/** A Stream info field a save names; Rust checks and sends only those. */
export type InfoField = 'title' | 'category' | 'tags' | 'language';

export type ManagerAction =
  | { kind: 'save_info'; draft: StreamInfoDraft; fields: InfoField[] }
  | { kind: 'create_poll'; title: string; choices: string[]; seconds: number; points_per_vote?: number | null }
  | { kind: 'end_poll'; show_results: boolean }
  | { kind: 'create_prediction'; title: string; outcomes: string[]; seconds: number }
  | { kind: 'lock_prediction' }
  | { kind: 'cancel_prediction' }
  | { kind: 'resolve_prediction'; outcome_id: string }
  | { kind: 'raid'; target_id: string; target_login: string; target_name: string }
  | { kind: 'cancel_raid' }
  | { kind: 'raid_now' }
  | { kind: 'run_ad'; seconds: number }
  | { kind: 'snooze_ad' }
  | { kind: 'redemptions'; ids: string[]; fulfill: boolean }
  | { kind: 'reward_queue'; reward_id: string; fulfill: boolean }
  | { kind: 'reward'; reward_id: string; paused?: boolean; enabled?: boolean }
  | { kind: 'chat_mode'; mode: ChatModeRow['kind']; on: boolean; value?: number | null }
  | { kind: 'shield'; on: boolean }
  | { kind: 'announce'; text: string; color?: string | null };

export interface ActOutcome {
  ok: boolean;
  message: string;
  view: StreamManagerView | null;
  queue: QueueView | null;
  /** The account the write ran as. */
  user_id: string | null;
}

export const getStreamManager = (channelId: string, channelLogin: string) =>
  invoke<StreamManagerView>('get_stream_manager', { channelId, channelLogin });

export const checkStreamInfo = (channelId: string, draft: StreamInfoDraft) =>
  invoke<InfoCheck>('check_stream_info', { channelId, draft });

/** The channel a write is for, and the account the form was showing. */
export interface ManagerTarget {
  channel_id: string;
  channel_login: string;
  /** The account the view was read as; Rust refuses the write when the
   *  signed-in account is another one by now. */
  user_id: string | null;
}

/** The target for a write made from `view`. */
export const targetOf = (view: StreamManagerView): ManagerTarget => ({
  channel_id: view.channel_id,
  channel_login: view.channel_login,
  user_id: view.user_id || null,
});

/** This window's hooks, told every act's answer for the channel it was sent to. */
const answerListeners = new Set<(channelId: string, outcome: ActOutcome) => void>();

/** One write through Rust's door, as the account the form was showing. The
 *  answer's view reaches this window's hooks before the caller sees it, so a
 *  result never shows over the view from before the write; other windows get
 *  Rust's push. */
export const managerAct = (target: ManagerTarget, action: ManagerAction) =>
  invoke<ActOutcome>('stream_manager_act', {
    channelId: target.channel_id,
    channelLogin: target.channel_login,
    action,
    expectedUserId: target.user_id,
  }).then((outcome) => {
    for (const l of [...answerListeners]) l(target.channel_id, outcome);
    return outcome;
  });

export const getRewardQueue = (channelId: string) => invoke<QueueView>('get_reward_queue', { channelId });

export const searchRaidTargets = (query: string, channelId: string) =>
  invoke<RaidTarget[]>('search_raid_targets', { query, channelId });

interface ManagerPush {
  channel_id: string;
  view: StreamManagerView;
  queue: QueueView | null;
}

// ─── the account ──────────────────────────────────────────────────────────────

/** The signed-in Twitch account's id from Rust's roster, null while it is
 *  unknown or not connected. */
export function twitchAccountOf(roster: AccountRoster | null): string | null {
  const row = roster?.main.find((a) => a.id === 'twitch');
  return row && row.status === 'connected' && row.account_id ? row.account_id : null;
}

/** The signed-in Twitch account, from the roster store every window seeds. */
export function useTwitchAccount(): string | null {
  return useAccountRosterStore((s) => twitchAccountOf(s.roster));
}

/**
 * Whether the account really changed: `next` is a known account that is not
 * the last known one. Unknown to known is not a change; A, then nothing, then
 * B is one.
 */
export function accountChanged(lastKnown: string | null, next: string | null): boolean {
  return lastKnown !== null && next !== null && lastKnown !== next;
}

// One listener per window: on a real account switch every draft, result and
// remembered page of the old account goes, so none of it shows to the next.
let lastKnownAccount: string | null = null;
function noteAccount(next: string | null): void {
  if (accountChanged(lastKnownAccount, next)) forgetAccount();
  if (next !== null) lastKnownAccount = next;
}
noteAccount(twitchAccountOf(useAccountRosterStore.getState().roster));
useAccountRosterStore.subscribe((s) => noteAccount(twitchAccountOf(s.roster)));

// ─── the hook's state ─────────────────────────────────────────────────────────

/** One hook's copy of a channel's view. Everything in it belongs to `channelId`
 *  and, once the account is known, to `account`. */
export interface ManagerState {
  channelId: string | null;
  account: string | null;
  view: StreamManagerView | null;
  queue: QueueView | null;
  error: string | null;
  /** The highest view and queue revisions taken here; an older one is dropped. */
  viewRev: number;
  queueRev: number;
  /** How many of this hook's reads had been sent when the last view landed. */
  viewSeq: number;
  /** Failed reads kept so far, so every failure (even with the same words) is seen. */
  errors: number;
}

export type ManagerEvent =
  | { type: 'switch'; channelId: string | null }
  | { type: 'account'; account: string | null }
  /** `sent`: how many of this hook's reads had been sent when it landed. */
  | { type: 'view'; channelId: string; view: StreamManagerView; sent: number }
  | { type: 'queue'; channelId: string; queue: QueueView }
  /** `seq`: the number of the read that failed. */
  | { type: 'error'; channelId: string; error: string; seq: number };

export const blankManager = (channelId: string | null, keep?: ManagerState): ManagerState => ({
  channelId,
  account: keep?.account ?? null,
  view: null,
  queue: null,
  error: null,
  // Rust's counter is global, so a revision seen on one channel still orders the next.
  viewRev: keep?.viewRev ?? 0,
  queueRev: keep?.queueRev ?? 0,
  viewSeq: keep?.viewSeq ?? 0,
  errors: keep?.errors ?? 0,
});

const otherAccount = (account: string | null, userId: string | undefined) =>
  account !== null && !!userId && userId !== account;

/**
 * The hook's state, keyed by channel and account. A switch forgets the old
 * channel at once; a read, answer or push for any other channel (a late
 * answer from the channel you left), for another account, or older than one
 * already shown (a read that started before a write and lands after it) is
 * dropped, so nothing stale or of another channel or account ever shows on,
 * or is written from, this one. A failed read is dropped once a view from a
 * later read has landed.
 */
export function managerState(s: ManagerState, e: ManagerEvent): ManagerState {
  if (e.type === 'switch') return e.channelId === s.channelId ? s : blankManager(e.channelId, s);
  if (e.type === 'account') {
    if (e.account === s.account) return s;
    // Unknown to known keeps a view that was read as that account.
    if (s.account === null && e.account !== null && !otherAccount(e.account, s.view?.user_id)) {
      const queue = otherAccount(e.account, s.queue?.user_id) ? null : s.queue;
      return { ...s, account: e.account, queue };
    }
    return { ...blankManager(s.channelId, s), account: e.account };
  }
  if (!s.channelId || e.channelId !== s.channelId) return s;
  switch (e.type) {
    case 'view': {
      const rev = e.view.rev ?? 0;
      if (e.view.channel_id !== s.channelId || rev < s.viewRev || otherAccount(s.account, e.view.user_id)) return s;
      return { ...s, view: e.view, error: null, viewRev: rev, viewSeq: Math.max(s.viewSeq, e.sent) };
    }
    case 'queue': {
      const rev = e.queue.rev ?? 0;
      if (rev < s.queueRev || otherAccount(s.account, e.queue.user_id)) return s;
      return { ...s, queue: e.queue, queueRev: rev };
    }
    case 'error':
      return e.seq <= s.viewSeq ? s : { ...s, error: e.error, errors: s.errors + 1 };
  }
}

/** How long to wait before reading again after the `attempt`th failure in a
 *  row (0 is the first): 5 s, 20 s, 60 s, then every 5 minutes for as long
 *  as the manager is in use. */
export function retryDelay(attempt: number): number {
  const steps = [5_000, 20_000, 60_000];
  return attempt < steps.length ? steps[Math.max(0, attempt)] : 300_000;
}

// ─── interest ─────────────────────────────────────────────────────────────────

/** When this page began, in whole milliseconds (`performance.timeOrigin`). */
const pageStart = (): number =>
  Math.floor(typeof performance !== 'undefined' && performance.timeOrigin ? performance.timeOrigin : Date.now());

/** A surface's name, unique across reloads of the same window: Rust keeps
 *  interest per window and surface, and a reloaded page must never reuse
 *  the last page's names. */
export const surfaceId = (pageStartMs: number, n: number): string => `sm-${Math.floor(pageStartMs)}-${n}`;

/** The first watch number a page sends: above any number an earlier page of
 *  the same window could have sent (up to a thousand calls per millisecond
 *  it lived), so Rust never takes a new page's call for an older one. Stays
 *  a safe integer for any real date. */
export const firstWatchSeq = (pageStartMs: number): number => Math.floor(pageStartMs) * 1000;

const PAGE_START = pageStart();
let surfaceCount = 0;
let watchSeq = firstWatchSeq(PAGE_START);

/** Tells Rust one surface (`surface`, unique in this page) is open for a
 *  channel, or no longer is. Later calls always carry a higher `seq`. */
export function sendWatch(surface: string, channelId: string, channelLogin: string, on: boolean): void {
  watchSeq += 1;
  void invoke('stream_manager_watch', { surface, channelId, channelLogin, on, seq: watchSeq }).catch(() => {});
}

/**
 * While `open`, this surface (a panel, the builder, the window, the Tools
 * menu, the phone's sheet) asks Rust to keep the channel's view current: Rust
 * re-reads it on its own and at every deadline, and pushes the answer as
 * `stream-manager-state`. Off on close, unmount, a channel or account change,
 * and when the page goes away. `pauseWhileHidden`: also off while the app is
 * in the background (the phone).
 */
export function useManagerInterest(
  open: boolean,
  channelId: string | null | undefined,
  channelLogin: string,
  opts: { pauseWhileHidden?: boolean } = {},
): void {
  const [surface] = useState(() => surfaceId(PAGE_START, (surfaceCount += 1)));
  const account = useTwitchAccount();
  const pause = !!opts.pauseWhileHidden;
  const [hidden, setHidden] = useState(false);
  useEffect(() => {
    if (!pause) return;
    // Read after lifecycle.ts has marked the app backgrounded for this change.
    const onChange = () => window.setTimeout(() => setHidden(isBackgrounded() || document.visibilityState === 'hidden'), 0);
    document.addEventListener('visibilitychange', onChange);
    return () => document.removeEventListener('visibilitychange', onChange);
  }, [pause]);
  const on = open && !!channelId && !(pause && hidden);
  useEffect(() => {
    if (!on || !channelId) return;
    sendWatch(surface, channelId, channelLogin, true);
    const gone = () => sendWatch(surface, channelId, channelLogin, false);
    window.addEventListener('pagehide', gone);
    return () => {
      window.removeEventListener('pagehide', gone);
      gone();
    };
  }, [on, channelId, channelLogin, account, surface]);
}

// ─── the hook ─────────────────────────────────────────────────────────────────

/**
 * The manager's view for a channel while `active`: read on activation, on an
 * account change and when `roleHint` changes, kept current by every window's
 * writes and Rust's own re-reads (`stream-manager-state`). A failed read is
 * tried again (`retryDelay`), and on focus or when the network comes back.
 * The view, queue and error are always this channel's and this account's: a
 * switch empties them until the new read lands.
 */
export function useStreamManager(
  channelId: string | null | undefined,
  channelLogin: string,
  active: boolean,
  roleHint = '',
) {
  const id = channelId || null;
  const account = useTwitchAccount();
  const [state, dispatch] = useReducer(managerState, id, (c) => blankManager(c));
  if (state.channelId !== id) dispatch({ type: 'switch', channelId: id });
  if (state.account !== account) dispatch({ type: 'account', account });
  // This render may still hold the old channel's copy; never hand it out.
  const current = state.channelId === id && state.account === account ? state : blankManager(id);
  const sent = useRef(0);
  const failures = useRef(0);

  const refresh = useCallback(async () => {
    if (!id) return;
    sent.current += 1;
    const seq = sent.current;
    try {
      const view = await getStreamManager(id, channelLogin);
      failures.current = 0;
      dispatch({ type: 'view', channelId: id, view, sent: sent.current });
    } catch (e) {
      dispatch({ type: 'error', channelId: id, error: typeof e === 'string' ? e : 'Could not read the channel', seq });
    }
  }, [id, channelLogin]);

  useEffect(() => {
    if (!active || !id) return;
    failures.current = 0;
    void refresh();
    const onAnswer = (ch: string, outcome: ActOutcome) => {
      if (outcome.view) dispatch({ type: 'view', channelId: ch, view: outcome.view, sent: sent.current });
      if (outcome.queue) dispatch({ type: 'queue', channelId: ch, queue: outcome.queue });
    };
    answerListeners.add(onAnswer);
    let off: (() => void) | undefined;
    let alive = true;
    void listen<ManagerPush>('stream-manager-state', (e) => {
      const ch = e.payload.channel_id;
      dispatch({ type: 'view', channelId: ch, view: e.payload.view, sent: sent.current });
      if (e.payload.queue) dispatch({ type: 'queue', channelId: ch, queue: e.payload.queue });
    }).then((u) => {
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      answerListeners.delete(onAnswer);
      off?.();
    };
  }, [active, id, refresh, account, roleHint]);

  // A failed read is tried again while the manager is in use.
  const failed = active && !!id && current.error !== null;
  useEffect(() => {
    if (!failed) return;
    const t = window.setTimeout(() => {
      failures.current += 1;
      void refresh();
    }, retryDelay(failures.current));
    const now = () => void refresh();
    window.addEventListener('focus', now);
    window.addEventListener('online', now);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener('focus', now);
      window.removeEventListener('online', now);
    };
  }, [failed, current.errors, refresh]);

  /** A queue read for `forChannel`. */
  const setQueue = useCallback((forChannel: string, queue: QueueView) => {
    dispatch({ type: 'queue', channelId: forChannel, queue });
  }, []);

  return { view: current.view, queue: current.queue, error: current.error, refresh, setQueue };
}

/** What `useStreamManager` hands out, for a surface given another's reads. */
export type StreamManagerHandle = ReturnType<typeof useStreamManager>;

/** `m:ss` left until `deadlineMs`, never below zero. */
export function countdown(deadlineMs: number, nowMs: number): string {
  const s = Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
