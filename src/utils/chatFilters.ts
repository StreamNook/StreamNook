// Chat filters for the in-app chat surfaces: hide messages from chosen users
// (per channel or everywhere) and, optionally, from known bots. The gate runs
// at chatConnectionStore ingest, so a hidden user's messages never enter the
// buffer and every consumer (main widget, MultiChat panes, blended panes, the
// live overlay feed) inherits the filter for free.
//
// Matching mirrors the stream overlay's blockedUsers dialect exactly:
// case-insensitive, leading @ stripped, and a name matches against the login
// OR the display name, so pasting either form works.
//
// Per-channel keys are COMPOSITE for every platform (`makeKey`, e.g.
// `twitch:xqc`), unlike the older bare-Twitch persisted key spaces elsewhere
// in the app: this is a brand-new persisted store with no legacy data to stay
// compatible with, and a composite key can never collide across platforms. Reads normalize through parseKey, so a bare
// login in a hand-edited settings file still resolves.

import { makeKey, parseKey } from './providerKey';
import { KNOWN_BOTS } from './knownBots';
import type { ProviderId } from '../types/providers';
import type { ChatFilterSettings } from '../types';

export function normalizeFilterName(name: string): string {
  return name.trim().toLowerCase().replace(/^@+/, '');
}

/** The composite per-channel key this feature stores lists under. */
export function filterChannelKey(provider: ProviderId, channel: string): string {
  return makeKey(provider, channel);
}

interface FilterIndex {
  global: Set<string>;
  perChannel: Map<string, Set<string>>;
  hideBots: boolean;
}

const EMPTY_INDEX: FilterIndex = { global: new Set(), perChannel: new Map(), hideBots: false };

// One index per settings object identity: settings updates replace the
// chat_filters object wholesale (the settings store never mutates nested
// objects in place), so a WeakMap keyed on it is a correct, self-evicting memo
// and the per-message cost is two Set lookups.
const indexCache = new WeakMap<ChatFilterSettings, FilterIndex>();

export function getFilterIndex(cf: ChatFilterSettings | null | undefined): FilterIndex {
  if (!cf) return EMPTY_INDEX;
  const hit = indexCache.get(cf);
  if (hit) return hit;
  const global = new Set<string>();
  for (const n of cf.hidden_users ?? []) {
    const norm = normalizeFilterName(n);
    if (norm) global.add(norm);
  }
  const perChannel = new Map<string, Set<string>>();
  const pc = cf.per_channel ?? {};
  for (const rawKey of Object.keys(pc)) {
    // Normalize the stored key through the codec so a bare legacy login and a
    // composite key land in the same bucket.
    const parsed = parseKey(rawKey);
    const key = makeKey(parsed.provider, parsed.channel);
    const set = perChannel.get(key) ?? new Set<string>();
    for (const n of pc[rawKey] ?? []) {
      const norm = normalizeFilterName(n);
      if (norm) set.add(norm);
    }
    if (set.size) perChannel.set(key, set);
    else perChannel.delete(key);
  }
  const built: FilterIndex = { global, perChannel, hideBots: cf.hide_bots === true };
  indexCache.set(cf, built);
  return built;
}

/** True when a message from this user should never enter the chat buffer. */
export function isFilteredChatUser(
  cf: ChatFilterSettings | null | undefined,
  provider: ProviderId,
  channel: string,
  username: string | undefined,
  displayName?: string,
): boolean {
  const idx = getFilterIndex(cf);
  if (!idx.hideBots && idx.global.size === 0 && idx.perChannel.size === 0) return false;
  const u = username ? normalizeFilterName(username) : '';
  const d = displayName ? normalizeFilterName(displayName) : '';
  if (!u && !d) return false;
  if (idx.hideBots && ((u && KNOWN_BOTS.has(u)) || (d && KNOWN_BOTS.has(d)))) return true;
  if ((u && idx.global.has(u)) || (d && idx.global.has(d))) return true;
  const set = idx.perChannel.get(filterChannelKey(provider, channel));
  if (!set) return false;
  return (!!u && set.has(u)) || (!!d && set.has(d));
}

/** Immutable helpers for the settings writers (UserProfileCard, ChatSettings). */
export function withHiddenUser(
  cf: ChatFilterSettings | undefined,
  name: string,
  scope: { provider: ProviderId; channel: string } | 'global',
  hidden: boolean,
): ChatFilterSettings {
  const norm = normalizeFilterName(name);
  const next: ChatFilterSettings = {
    ...cf,
    hidden_users: [...(cf?.hidden_users ?? [])],
    per_channel: { ...(cf?.per_channel ?? {}) },
  };
  if (scope === 'global') {
    const list = next.hidden_users!.filter((n) => normalizeFilterName(n) !== norm);
    if (hidden) list.push(norm);
    next.hidden_users = list;
  } else {
    const key = filterChannelKey(scope.provider, scope.channel);
    const list = (next.per_channel![key] ?? []).filter((n) => normalizeFilterName(n) !== norm);
    if (hidden) list.push(norm);
    if (list.length) next.per_channel![key] = list;
    else delete next.per_channel![key];
  }
  return next;
}

export function isHiddenInScope(
  cf: ChatFilterSettings | undefined,
  name: string,
  scope: { provider: ProviderId; channel: string } | 'global',
): boolean {
  const idx = getFilterIndex(cf);
  const norm = normalizeFilterName(name);
  if (!norm) return false;
  if (scope === 'global') return idx.global.has(norm);
  return idx.perChannel.get(filterChannelKey(scope.provider, scope.channel))?.has(norm) ?? false;
}
