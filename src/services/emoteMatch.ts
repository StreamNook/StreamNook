// The composer's emote matching lives in Rust (`match_emote_tokens`), over the
// sets Rust already holds. This file is the typed bridge: it passes what the
// page knows (the channel, what was typed, the 7TV size it renders) and shapes
// rows for the Tab cycle and the emote list. No ranking happens here.
import { invoke, convertFileSrc } from '@tauri-apps/api/core';

import { inlineEmoteTier } from './emoteService';
import type { ProviderId } from '../types/providers';
import type { EmoteTabCandidate } from '../utils/chatInputWord';

export type EmoteMatchProfile = 'cycle' | 'search';

export interface EmoteMatchRow {
  id: string;
  name: string;
  /** What goes into the message when it differs from the name. */
  insertText?: string;
  url: string;
  localUrl?: string;
  provider: NonNullable<EmoteTabCandidate['emote']>['provider'];
  /** Where the emote comes from: "Twitch Global", "Channel Sub", "7TV", ... */
  sourceLabel: string;
  /** The same without the provider ("Global", "Channel Sub", "Personal"), for
   *  surfaces that show the provider as its logo. */
  sourceDetail: string;
  /** Where the typed text sits in the name, for highlighting. */
  matchAt?: number;
  matchLen?: number;
  /** Section heading while browsing with nothing typed. */
  group?: string;
  isZeroWidth?: boolean;
  modifierFlags?: number;
}

export interface EmoteMatchResult {
  rows: EmoteMatchRow[];
  /** Matches in all, before the row cap. */
  total: number;
  /** False while the channel's emotes are still loading. */
  ready: boolean;
}

export interface EmoteMatchChannel {
  provider: ProviderId;
  /** The login, slug or identifier the channel's emotes were fetched with. */
  channel: string;
  channelId?: string | null;
}

interface WireRow {
  id: string;
  name: string;
  insert_text?: string;
  url: string;
  local_path?: string;
  provider: EmoteMatchRow['provider'];
  source_label: string;
  source_detail: string;
  match_at?: number;
  match_len?: number;
  group?: string;
  is_zero_width?: boolean;
  modifier_flags?: number;
}

const EMPTY: EmoteMatchResult = { rows: [], total: 0, ready: false };

export async function matchEmotes(
  target: EmoteMatchChannel,
  query: string,
  profile: EmoteMatchProfile,
  options: { twitchFirst?: boolean; limit?: number } = {},
): Promise<EmoteMatchResult> {
  if (!target.channel) return EMPTY;
  try {
    const res = await invoke<{ rows: WireRow[]; total: number; ready: boolean }>('match_emote_tokens', {
      provider: target.provider,
      channel: target.channel,
      channelId: target.channelId || null,
      query,
      profile,
      order: options.twitchFirst ? 'twitch_first' : 'default',
      tier: inlineEmoteTier(),
      limit: options.limit ?? null,
    });
    return {
      total: res.total,
      ready: res.ready,
      rows: res.rows.map((r) => ({
        id: r.id,
        name: r.name,
        insertText: r.insert_text,
        url: r.url,
        localUrl: r.local_path ? convertFileSrc(r.local_path) : undefined,
        provider: r.provider,
        sourceLabel: r.source_label,
        sourceDetail: r.source_detail,
        matchAt: r.match_at,
        matchLen: r.match_len,
        group: r.group,
        isZeroWidth: r.is_zero_width,
        modifierFlags: r.modifier_flags,
      })),
    };
  } catch {
    return EMPTY;
  }
}

/** The viewer's personal 7TV emote that shows instead of a channel emote of
 *  the same name in their own messages. */
export interface PersonalOverride {
  name: string;
  id: string;
  url: string;
}

/** Channel emote names the viewer's personal set replaces, keyed by name.
 *  Twitch only (personal emotes are a Twitch feature); empty until Rust knows
 *  both sets. */
export async function personalEmoteOverrides(
  channel: string,
  channelId?: string | null,
): Promise<Map<string, PersonalOverride>> {
  try {
    const list = await invoke<PersonalOverride[]>('personal_emote_overrides', {
      channel,
      channelId: channelId || null,
    });
    return new Map(list.map((o) => [o.name, o]));
  } catch {
    return new Map();
  }
}

/** Rows as Tab-cycle candidates, in the order Rust ranked them. */
export function rowsToTabCandidates(rows: EmoteMatchRow[]): EmoteTabCandidate[] {
  return rows.map((r, i) => ({
    name: r.name,
    insertText: r.insertText,
    priority: i,
    emote: {
      id: r.id,
      name: r.name,
      url: r.url,
      localUrl: r.localUrl,
      provider: r.provider,
      isZeroWidth: r.isZeroWidth,
      modifierFlags: r.modifierFlags,
    },
    sourceLabel: r.sourceLabel,
    sourceDetail: r.sourceDetail,
  }));
}

/**
 * A request counter for one input: `next()` claims a number, and a reply is
 * applied only while `isCurrent` still holds for it, so a slow answer to an
 * older keystroke never overwrites a newer one.
 */
export function createRequestSeq() {
  let seq = 0;
  return {
    next: () => ++seq,
    isCurrent: (n: number) => n === seq,
  };
}
