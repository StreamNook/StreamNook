// Frontend side of the 7TV EventAPI bridge.
//
// The Rust seventv_eventapi service holds one shared WebSocket and emits
// `7tv://emote-set-update` to every window when a channel's 7TV emote set
// changes live. Each window applies it to ITS OWN per-window emote cache and
// injects the in-chat notice into its own chat store, so main and any MultiChat
// popout showing the channel both update independently.
//
// The payload carries the COMPOSED delta (rows to drop, rows to add), computed
// once in Rust with the dictionary's precedence rules, so a window patches its
// cached set in place and fetches nothing. The old path refetched the whole
// channel document per change; on a large channel that is 14 MB, and when the
// fetch failed the picker was left holding a globals-only set (2026-09-07).

import {
  applyChannelEmoteDelta,
  getChannelEmotes,
  injectSystemMessage,
  refreshChannelEmotes,
  systemSourceFor,
} from '../stores/chatConnectionStore';
import type { Emote } from '../services/emoteService';
import type { ProviderId } from '../types/providers';
import { forceRefreshCosmetics } from '../services/cosmeticsCache';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';

export interface EmoteSetUpdatePayload {
  channel: string; // lowercase channel key (login / slug / identifier)
  channel_id: string; // the platform's own channel id
  platform?: string; // 'twitch' (default) | 'kick' | 'youtube'
  actor_name: string;
  added: string[];
  removed: string[];
  renamed: { old: string; new: string }[];
  /** Composed dictionary delta from Rust: rows to drop (by id AND name) then
   *  rows to add, including any global a removal stopped shadowing. Null when
   *  Rust holds no copy of the set (no chat open on it there); the window then
   *  refetches, and the Rust cache was invalidated for that. */
  composed?: { added: Emote[]; removed: { id: string; name: string }[] } | null;
}

/**
 * Apply a live 7TV emote-set change to the current window: patch the channel's
 * cached set from the composed delta (so the picker, autocomplete, and the
 * local-echo fallback pick up the change) and, if enabled, drop a notice line in
 * chat for each add/remove/rename. Incoming chat needs nothing here: Rust
 * tokenizes it against a dictionary it already patched.
 */
export async function handleSeventvEmoteSetUpdate(payload: EmoteSetUpdatePayload): Promise<void> {
  const { channel, channel_id, actor_name, added, removed, renamed } = payload;
  // 7TV supports all three platforms, so an update can be for any of them. The
  // chat slice is keyed by the composite key for a provider and by the bare
  // login for Twitch; the emote cache is keyed by emoteCacheKey, which the store
  // helpers derive from (channel, platform) themselves.
  const platform = (payload.platform || 'twitch') as ProviderId;
  const chatKey = platform === 'twitch' ? channel : `${platform}:${channel}`;

  let patched = false;
  if (payload.composed) {
    try {
      patched = applyChannelEmoteDelta(channel, platform, payload.composed);
    } catch (e) {
      Logger.warn('[7TV EventAPI] failed to patch emotes for', channel, e);
    }
  }
  // No composed delta (Kick and YouTube refetch in Rust; or Rust held no copy)
  // and this window has a set cached: that set is stale, refetch it. The Rust
  // cache was refreshed or invalidated, so this is one round trip to it. A
  // window with nothing cached has nothing to update.
  if (!patched && getChannelEmotes(channel, platform)) {
    try {
      await refreshChannelEmotes(channel, channel_id, platform);
    } catch (e) {
      Logger.warn('[7TV EventAPI] failed to refresh emotes for', channel, e);
    }
  }

  const noticesEnabled =
    useAppStore.getState().settings.chat_design?.seventv_emote_notices ?? true;
  if (!noticesEnabled) return;

  const actor = actor_name || 'Someone';
  const source = systemSourceFor(chatKey);
  for (const name of added) {
    injectSystemMessage(chatKey, `${actor} added the emote ${name}`, undefined, source);
  }
  for (const name of removed) {
    injectSystemMessage(chatKey, `${actor} removed the emote ${name}`, undefined, source);
  }
  for (const r of renamed) {
    injectSystemMessage(chatKey, `${actor} renamed the emote ${r.old} to ${r.new}`, undefined, source);
  }
}

export interface CosmeticUpdatePayload {
  twitch_id: string;
  action: string; // 'create' | 'update' | 'delete'
}

/**
 * A present user's 7TV cosmetics changed (delivered live over the EventAPI).
 * We re-resolve the authoritative cosmetics through the existing v4 GQL path
 * (correct render shape, cached + coalesced), which publishes into the shared
 * cosmetics cache; the chatUserStore bridge then repaints their chat row if
 * they are visible, and a not-yet-seen user gets painted instantly on their
 * first message (the fetch pre-warmed the cache). The WS is the trigger; GQL
 * is the resolver.
 */
export async function handleSeventvCosmeticUpdate(payload: CosmeticUpdatePayload): Promise<void> {
  const { twitch_id } = payload;
  if (!twitch_id) return;

  // The WS says this user's cosmetics exist or just changed. Force a genuinely
  // fresh resolve (clears BOTH cache layers, incl. the lower-level 7TV
  // userCache) for every action: a 'create' can race a poisoned success-empty
  // from an earlier fetch, and 'update'/'delete' changed the selection. A plain
  // invalidate didn't reach the 7TV layer, so the stale entry was served for
  // its TTL and the change never showed.
  try {
    await forceRefreshCosmetics(twitch_id);
  } catch (e) {
    Logger.warn('[7TV EventAPI] cosmetics resolve failed for', twitch_id, e);
  }
}
