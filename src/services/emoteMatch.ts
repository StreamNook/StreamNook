// The composer's emote matching lives in Rust (`match_emote_tokens`), over the
// sets Rust already holds. This file is the typed bridge: it passes what the
// page knows (the channel, what was typed, the 7TV size it renders) and shapes
// rows for the Tab cycle and the emote list. No ranking happens here.
import { invoke, convertFileSrc } from '@tauri-apps/api/core';

import { inlineEmoteTier } from './emoteService';
import type { ProviderId } from '../types/providers';
import type { EmoteTabCandidate } from '../utils/chatInputWord';

export type EmoteMatchProfile = 'cycle' | 'search' | 'fill';

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
  /** Set on a chatter row (Rust's provider `user`), which follows the emotes
   *  in a cycle answer: the chatter's login. */
  chatterLogin?: string;
  isZeroWidth?: boolean;
  modifierFlags?: number;
  emoteType?: string;
  /** Who made it, where the provider's set says (the menu's hover credit). */
  ownerName?: string;
  /** The emote's own name when the set shows it under an alias. */
  originalName?: string;
  animated?: boolean;
  width?: number;
  favorite?: boolean;
  /** An FFZ subscriber effect this account cannot use (picker mode only). */
  locked?: boolean;
}

export interface EmoteMatchResult {
  rows: EmoteMatchRow[];
  /** Matches in all, before the row cap. */
  total: number;
  /** False while the channel's emotes are still loading. */
  ready: boolean;
  /** Matches per provider before the cap (`twitch`, `7tv`, `bttv`, ...). */
  counts: Partial<Record<EmoteMatchRow['provider'], number>>;
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
  provider: EmoteMatchRow['provider'] | 'user';
  source_label: string;
  source_detail: string;
  match_at?: number;
  match_len?: number;
  group?: string;
  is_zero_width?: boolean;
  modifier_flags?: number;
  emote_type?: string;
  owner_name?: string;
  original_name?: string;
  animated?: boolean;
  width?: number;
  favorite?: boolean;
  locked?: boolean;
  login?: string;
}

const EMPTY: EmoteMatchResult = { rows: [], total: 0, ready: false, counts: {} };

export async function matchEmotes(
  target: EmoteMatchChannel,
  query: string,
  profile: EmoteMatchProfile,
  /** `picker`: the emote menu's search, which also lists FFZ subscriber
   *  effects this account cannot send (marked `locked`). `typing`: asked as
   *  the viewer types, not on Tab (Rust's `MIN_PREFIX_CHARS`, one letter). */
  options: { twitchFirst?: boolean; limit?: number; picker?: boolean; typing?: boolean } = {},
): Promise<EmoteMatchResult> {
  if (!target.channel) return EMPTY;
  try {
    const res = await invoke<{ rows: WireRow[]; total: number; ready: boolean; counts?: EmoteMatchResult['counts'] }>('match_emote_tokens', {
      provider: target.provider,
      channel: target.channel,
      channelId: target.channelId || null,
      query,
      profile,
      order: options.twitchFirst ? 'twitch_first' : 'default',
      tier: inlineEmoteTier(),
      limit: options.limit ?? null,
      picker: options.picker ?? null,
      typing: options.typing ?? null,
    });
    return {
      total: res.total,
      ready: res.ready,
      counts: res.counts ?? {},
      rows: res.rows.map((r) => ({
        id: r.id,
        name: r.name,
        insertText: r.insert_text,
        url: r.url,
        localUrl: r.local_path ? convertFileSrc(r.local_path) : undefined,
        provider: r.provider === 'user' ? 'twitch' : r.provider,
        sourceLabel: r.source_label,
        sourceDetail: r.source_detail,
        matchAt: r.match_at,
        matchLen: r.match_len,
        group: r.group,
        isZeroWidth: r.is_zero_width,
        modifierFlags: r.modifier_flags,
        emoteType: r.emote_type,
        ownerName: r.owner_name,
        originalName: r.original_name,
        animated: r.animated,
        width: r.width,
        favorite: r.favorite,
        locked: r.locked,
        chatterLogin: r.provider === 'user' ? (r.login ?? r.name.replace(/^@/, '')) : undefined,
      })),
    };
  } catch {
    return EMPTY;
  }
}

/**
 * The emote fill menu: the desktop's colon list and both phones' suggestion
 * strip ask Rust the same question (`profile` "fill": ranked by closeness,
 * Twitch first, as the viewer types), so one word gets one list on every app.
 * `word` is what was typed, a desktop colon included (a colon word lists no
 * chatters after the emotes).
 */
export function matchEmoteFill(target: EmoteMatchChannel, word: string): Promise<EmoteMatchResult> {
  return matchEmotes(target, word, 'fill');
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

/** Rows as Tab-cycle candidates, in the order Rust ranked them. Chatter rows
 *  (provider `user`, after the emotes) insert what Rust chose: the login when
 *  a display name is localized. */
export function rowsToTabCandidates(rows: EmoteMatchRow[]): EmoteTabCandidate[] {
  return rows.map((r, i): EmoteTabCandidate => {
    if (r.chatterLogin !== undefined) {
      return {
        name: r.name,
        insertText: r.insertText,
        priority: i,
        chatter: { username: r.chatterLogin, displayName: r.name.replace(/^@/, '') },
      };
    }
    return {
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
    };
  });
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
