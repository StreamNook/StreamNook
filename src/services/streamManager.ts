// The stream manager's Rust view and door, typed for the desktop and Android
// shells. Rust decides who may do what, words every line and runs every write
// (services/stream_manager.rs); these types only mirror what it sends.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useReducer } from 'react';

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
}

export interface RunningPrediction {
  id: string;
  title: string;
  status: 'active' | 'locked';
  outcomes: { id: string; title: string; color: string }[];
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

export type ManagerAction =
  | { kind: 'save_info'; draft: StreamInfoDraft }
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
}

export const getStreamManager = (channelId: string, channelLogin: string) =>
  invoke<StreamManagerView>('get_stream_manager', { channelId, channelLogin });

export const checkStreamInfo = (channelId: string, draft: StreamInfoDraft) =>
  invoke<InfoCheck>('check_stream_info', { channelId, draft });

export const managerAct = (channelId: string, channelLogin: string, action: ManagerAction) =>
  invoke<ActOutcome>('stream_manager_act', { channelId, channelLogin, action });

export const getRewardQueue = (channelId: string) => invoke<QueueView>('get_reward_queue', { channelId });

export const searchRaidTargets = (query: string, channelId: string) =>
  invoke<RaidTarget[]>('search_raid_targets', { query, channelId });

interface ManagerPush {
  channel_id: string;
  view: StreamManagerView;
  queue: QueueView | null;
}

/** One hook's copy of a channel's view. Everything in it belongs to `channelId`. */
export interface ManagerState {
  channelId: string | null;
  view: StreamManagerView | null;
  queue: QueueView | null;
  error: string | null;
}

export type ManagerEvent =
  | { type: 'switch'; channelId: string | null }
  | { type: 'view'; channelId: string; view: StreamManagerView }
  | { type: 'queue'; channelId: string; queue: QueueView }
  | { type: 'error'; channelId: string; error: string };

const blank = (channelId: string | null): ManagerState => ({ channelId, view: null, queue: null, error: null });

/**
 * The hook's state, keyed by channel. A switch forgets the old channel at
 * once; a read, answer or push for any other channel than the current one
 * (a late answer from the channel you left) is dropped, so nothing of that
 * channel ever shows on, or is written from, this one.
 */
export function managerState(s: ManagerState, e: ManagerEvent): ManagerState {
  if (e.type === 'switch') return e.channelId === s.channelId ? s : blank(e.channelId);
  if (!s.channelId || e.channelId !== s.channelId) return s;
  switch (e.type) {
    case 'view':
      return e.view.channel_id === s.channelId ? { ...s, view: e.view, error: null } : s;
    case 'queue':
      return { ...s, queue: e.queue };
    case 'error':
      return { ...s, error: e.error };
  }
}

/**
 * The manager's view for a channel while `active`: read on activation, kept
 * current by every window's writes (`stream-manager-state`). The view, queue
 * and error are always this channel's: a switch empties them until the new
 * channel's read lands.
 */
export function useStreamManager(channelId: string | null | undefined, channelLogin: string, active: boolean) {
  const id = channelId || null;
  const [state, dispatch] = useReducer(managerState, id, blank);
  if (state.channelId !== id) dispatch({ type: 'switch', channelId: id });
  // This render still holds the old channel's copy; never hand it out.
  const current = state.channelId === id ? state : blank(id);

  const refresh = useCallback(async () => {
    if (!id) return;
    try {
      const view = await getStreamManager(id, channelLogin);
      dispatch({ type: 'view', channelId: id, view });
    } catch (e) {
      dispatch({ type: 'error', channelId: id, error: typeof e === 'string' ? e : 'Could not read the channel' });
    }
  }, [id, channelLogin]);

  useEffect(() => {
    if (!active || !id) return;
    void refresh();
    let off: (() => void) | undefined;
    let alive = true;
    void listen<ManagerPush>('stream-manager-state', (e) => {
      const ch = e.payload.channel_id;
      dispatch({ type: 'view', channelId: ch, view: e.payload.view });
      if (e.payload.queue) dispatch({ type: 'queue', channelId: ch, queue: e.payload.queue });
    }).then((u) => {
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [active, id, refresh]);

  /** An action's answer for `forChannel` (the channel it was sent to). */
  const apply = useCallback((outcome: ActOutcome, forChannel: string) => {
    if (outcome.view) dispatch({ type: 'view', channelId: forChannel, view: outcome.view });
    if (outcome.queue) dispatch({ type: 'queue', channelId: forChannel, queue: outcome.queue });
  }, []);

  /** A queue read for `forChannel`. */
  const setQueue = useCallback((forChannel: string, queue: QueueView) => {
    dispatch({ type: 'queue', channelId: forChannel, queue });
  }, []);

  return { view: current.view, queue: current.queue, error: current.error, refresh, apply, setQueue };
}

/** `m:ss` left until `deadlineMs`, never below zero. */
export function countdown(deadlineMs: number, nowMs: number): string {
  const s = Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
