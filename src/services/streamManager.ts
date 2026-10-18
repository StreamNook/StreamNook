// The stream manager's Rust view and door, typed for the desktop and Android
// shells. Rust decides who may do what, words every line and runs every write
// (services/stream_manager.rs); these types only mirror what it sends.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useState } from 'react';

export type ManagerRole = 'broadcaster' | 'editor' | 'moderator' | 'moderator_editor' | 'viewer';
export type ManagerSection = 'info' | 'poll' | 'prediction' | 'raid' | 'ads' | 'rewards' | 'chat' | 'announce';

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

/**
 * The manager's view for a channel while `active`: read on activation, kept
 * current by every window's writes (`stream-manager-state`). `apply` takes the
 * view an action answered with.
 */
export function useStreamManager(channelId: string | null | undefined, channelLogin: string, active: boolean) {
  const [view, setView] = useState<StreamManagerView | null>(null);
  const [queue, setQueue] = useState<QueueView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!channelId) return;
    try {
      setView(await getStreamManager(channelId, channelLogin));
      setError(null);
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Could not read the channel');
    }
  }, [channelId, channelLogin]);

  useEffect(() => {
    if (!active || !channelId) return;
    void refresh();
    let off: (() => void) | undefined;
    let alive = true;
    void listen<ManagerPush>('stream-manager-state', (e) => {
      if (e.payload.channel_id !== channelId) return;
      setView(e.payload.view);
      if (e.payload.queue) setQueue(e.payload.queue);
    }).then((u) => {
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [active, channelId, refresh]);

  const apply = useCallback((outcome: ActOutcome) => {
    if (outcome.view) setView(outcome.view);
    if (outcome.queue) setQueue(outcome.queue);
  }, []);

  return { view, queue, setQueue, error, refresh, apply };
}

/** `m:ss` left until `deadlineMs`, never below zero. */
export function countdown(deadlineMs: number, nowMs: number): string {
  const s = Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
