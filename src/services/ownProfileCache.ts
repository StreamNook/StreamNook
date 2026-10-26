// Synchronous, local snapshot of the signed-in member's Profile -> Overview
// render inputs (paint, badges, loadout, theme). The in-memory cosmetics and
// identity caches are wiped on every app restart, so a cold open of the Profile
// pane used to paint a barebones card (no paint, no badges) and then "dress up"
// as each async source resolved a beat later. This persists the last rendered
// loadout to localStorage so the card paints fully-dressed on the next open
// BEFORE any network read; the normal background fetches then revalidate and
// overwrite only what actually changed.
//
// Not a source of truth: 7TV / Twitch / the Identity API stay authoritative, and
// a stale entry self-corrects on the next revalidation. It is written through on
// every change to the rendered cosmetics, so opening the profile after editing a
// loadout always reflects the latest selection.

import { Logger } from '../utils/logger';
import { withoutLoopbackUrls } from '../utils/cacheFileUrl';

const KEY = 'streamnook_own_profile_v1';
// Signed-in user plus a few linked accounts. Each snapshot is a few KB of JSON
// (image URLs, not blobs), so this stays well under the localStorage quota.
const MAX_ENTRIES = 4;

export interface OwnProfileSnapshot {
  seventvPaint: unknown | null;
  seventvBadges: unknown[];
  allSeventvPaints: unknown[];
  seventvUserId: string | null;
  twitchBadges: unknown[];
  thirdPartyBadges: unknown[];
  chatIdentityBadges: unknown[];
  selfBttvProBadge: unknown | null;
  loadout: { customized: boolean; badges: string[] };
  profileTheme: string;
  hiddenSections: string[];
}

type Store = Record<string, OwnProfileSnapshot>;

const readStore = (): Store => {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as Store) : {};
  } catch {
    return {};
  }
};

/** Synchronous read of a member's cached profile snapshot, or null. */
export function readOwnProfileCache(userId: string | null | undefined): OwnProfileSnapshot | null {
  if (!userId) return null;
  return readStore()[userId] ?? null;
}

/**
 * Persist a member's profile snapshot (write-through). Bounds the store to the
 * MAX_ENTRIES most-recently-written members; object key order is insertion order,
 * so re-adding the touched user last makes it the newest and evicts the oldest.
 */
export function writeOwnProfileCache(
  userId: string | null | undefined,
  snapshot: OwnProfileSnapshot,
): void {
  if (!userId) return;
  try {
    const store = readStore();
    delete store[userId]; // re-insert at the end to mark most-recently-written
    const others = Object.entries(store);
    const kept = others.slice(Math.max(0, others.length - (MAX_ENTRIES - 1)));
    const next: Store = {};
    for (const [id, snap] of kept) next[id] = snap;
    next[userId] = snapshot;
    // A loopback image URL dies with this launch (new port and token): never saved.
    localStorage.setItem(KEY, JSON.stringify(next, withoutLoopbackUrls));
  } catch (e) {
    Logger.warn('[ownProfileCache] write failed:', e);
  }
}
