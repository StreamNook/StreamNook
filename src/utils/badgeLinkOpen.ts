// Opening a badge from a `streamnook:///badge/<set>[/<version>]` link on desktop.
// Rust parses and cleans the link (`services::badge_link`); this file routes it:
// the detail opens at once on a placeholder and the lookup only fills it.
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import type { SelectedBadge } from '../components/BadgeDetailSlot';
import { Logger } from './logger';

/** The badge a link names: set id (lowercase) and version. */
export interface BadgeLinkTarget {
  setId: string;
  version: string;
}

/** One version of a Twitch badge set, as Twitch's list carries it. */
export interface BadgeVersion {
  id: string;
  image_url_1x: string;
  image_url_2x: string;
  image_url_4x: string;
  title: string;
  description: string;
  click_action: string | null;
  click_url: string | null;
  set_id?: string;
}

/** What the sheet says when Twitch's list was read and has no such badge. */
export const BADGE_NOT_FOUND_TEXT = "Couldn't find this badge yet";

/** What the sheet says when the list could not be read at all (offline, a rejected read, too slow). */
export const BADGE_LOAD_FAILED_TEXT = "Couldn't load this badge";

/** Android's RewardsScreen still imports the old toast name; its slice deletes this alias. */
export const BADGE_NOT_FOUND_TOAST = BADGE_NOT_FOUND_TEXT;

/** How long a badge link waits for the list before the sheet says it failed. */
export const BADGE_LINK_LIMIT_MS = 6000;

/** What a badge link's lookup found, for the sheet that is already open on its placeholder. */
export type BadgeLinkAnswer =
  | { kind: 'badge'; badge: BadgeVersion; setId: string }
  | { kind: 'link'; key: string; state: 'missing' | 'failed' };

/** What `openBadgeLink` needs from the app; stubbed in tests. */
export interface BadgeLinkDeps {
  /** True while the player (or a MultiNook tile) is full screen. */
  isPlayerFullscreen: () => boolean;
  /** Leaves the player's full screen (`exitPlayerFullscreen`). */
  exitPlayerFullscreen: () => void;
  /** Opens the badge detail on a loading placeholder for `key` and returns the number of this read. */
  showBadgeLink: (key: string) => number;
  /** Hands the lookup's answer to the sheet; the sheet applies it only while it still shows `key` and `read`. */
  answerBadgeLink: (key: string, read: number, next: BadgeLinkAnswer) => void;
}

/** Read what `take_pending_badge_link` returned; null when it is not a link. */
export function readBadgeLink(raw: unknown): BadgeLinkTarget | null {
  if (!raw || typeof raw !== 'object') return null;
  const { setId, version } = raw as { setId?: unknown; version?: unknown };
  if (typeof setId !== 'string' || !setId) return null;
  return { setId, version: typeof version === 'string' && version ? version : '1' };
}

interface VersionLike {
  id: string;
}

interface SetLike<V extends VersionLike> {
  set_id: string;
  versions: V[];
}

/**
 * The version a link names inside Twitch's badge sets: set id compared
 * case-insensitively, version exactly. Returns the version carrying its set id.
 */
export function findTwitchBadge<V extends VersionLike>(
  sets: ReadonlyArray<SetLike<V>>,
  setId: string,
  version: string,
): (V & { set_id: string }) | undefined {
  const wanted = setId.toLowerCase();
  for (const set of sets) {
    if (set.set_id.toLowerCase() !== wanted) continue;
    const hit = set.versions.find((v) => v.id === version);
    if (hit) return { ...hit, set_id: set.set_id };
  }
  return undefined;
}

/** Split a `set/version` key. The version is whatever follows the last slash; a key with none has an empty version. */
export function splitBadgeKey(key: string): BadgeLinkTarget {
  const at = key.lastIndexOf('/');
  return at < 0 ? { setId: key, version: '' } : { setId: key.slice(0, at), version: key.slice(at + 1) };
}

interface TwitchBadgeList {
  data: Array<SetLike<BadgeVersion>>;
}

const asList = (raw: unknown): TwitchBadgeList | null =>
  raw && typeof raw === 'object' && Array.isArray((raw as TwitchBadgeList).data) ? (raw as TwitchBadgeList) : null;

const hitAnswer = (hit: BadgeVersion & { set_id: string }): BadgeLinkAnswer => ({
  kind: 'badge',
  badge: hit,
  setId: hit.set_id,
});

/** The list as already cached, or null when there is none (or the read was rejected). */
async function readCachedList(): Promise<TwitchBadgeList | null> {
  try {
    return asList(await invoke<unknown>('get_cached_global_badges'));
  } catch (err) {
    Logger.warn('[badgeLink] Cached badge list unavailable:', err);
    return null;
  }
}

async function readBadgeLinkAnswer(key: string, setId: string, version: string): Promise<BadgeLinkAnswer> {
  const missing: BadgeLinkAnswer = { kind: 'link', key, state: 'missing' };
  const failed: BadgeLinkAnswer = { kind: 'link', key, state: 'failed' };

  const cached = await readCachedList();
  const cachedHit = cached ? findTwitchBadge(cached.data, setId, version) : undefined;
  if (cachedHit) return hitAnswer(cachedHit);

  // The forced refresh needs a token: signed out, the cached list alone decides.
  if (!useAppStore.getState().isAuthenticated) return cached ? missing : failed;

  try {
    const refreshed = asList(await invoke<unknown>('force_refresh_global_badges'));
    if (!refreshed) return failed;
    const hit = findTwitchBadge(refreshed.data, setId, version);
    return hit ? hitAnswer(hit) : missing;
  } catch (err) {
    Logger.warn('[badgeLink] Badge list refresh failed:', err);
    return failed;
  }
}

/**
 * Look up the badge a `set/version` key names: the cached list first, then one forced refresh.
 * A hit answers with the badge, a list that was read and has no match answers `missing`, and a
 * rejected read or no list at all answers `failed`.
 *
 * `onAnswer` can be called twice: a 6 s timer answers `failed` without canceling the read, and the
 * read's own completion answers whenever it lands, so a slow refresh still fills a sheet that has
 * already said it failed. The sheet decides which answer applies.
 */
export function resolveBadgeLink(key: string, onAnswer: (next: BadgeLinkAnswer) => void): void {
  const { setId, version } = splitBadgeKey(key);

  const timer = setTimeout(() => onAnswer({ kind: 'link', key, state: 'failed' }), BADGE_LINK_LIMIT_MS);
  readBadgeLinkAnswer(key, setId, version).then(
    (answer) => {
      clearTimeout(timer);
      onAnswer(answer);
    },
    (err) => {
      clearTimeout(timer);
      Logger.warn('[badgeLink] Badge lookup failed:', err);
      onAnswer({ kind: 'link', key, state: 'failed' });
    },
  );
}

/**
 * The slot after a lookup answered, given the slot as it is now. An answer applies only while the
 * slot still shows the same key and the same read: a loading placeholder takes any answer, one that
 * already said failed (the 6 s timer fired first) takes a late hit, and a link answer keeps the
 * slot's read so the timer's failed still matches the read's own late hit. Anything else, an answer
 * for another key or an older read or a slot that was closed, writes nothing.
 */
export function applyBadgeAnswer(cur: SelectedBadge, key: string, read: number, next: BadgeLinkAnswer): SelectedBadge {
  if (cur?.kind !== 'link' || cur.key !== key || cur.read !== read) return cur;
  if (cur.state === 'loading' || (cur.state === 'failed' && next.kind === 'badge')) {
    return next.kind === 'link' ? { ...next, read } : next;
  }
  return cur;
}

/**
 * Open the link's badge on desktop: leave full screen first (only when the player is in it, so the
 * overlays open in the window as they always do), show the detail on a placeholder at once, and let
 * the lookup fill it.
 */
export function openBadgeLink(link: BadgeLinkTarget, deps: BadgeLinkDeps): void {
  if (deps.isPlayerFullscreen()) deps.exitPlayerFullscreen();
  const key = `${link.setId}/${link.version}`;
  const read = deps.showBadgeLink(key);
  resolveBadgeLink(key, (next) => deps.answerBadgeLink(key, read, next));
}
