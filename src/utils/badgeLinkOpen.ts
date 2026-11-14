// Opening a badge from a `streamnook:///badge/<set>[/<version>]` link, shared by
// the desktop shell (App.tsx) and the BadgesOverlay's lookup. Rust parses and
// cleans the link (`services::badge_link`); this file only routes it.

/** The badge a link names: set id (lowercase) and version. */
export interface BadgeLinkTarget {
  setId: string;
  version: string;
}

/** The toast both platforms show when the badge is not in Twitch's list. */
export const BADGE_NOT_FOUND_TOAST = "Couldn't find this badge yet";

/** What `openBadgeLink` needs from the app; stubbed in tests. */
export interface BadgeLinkDeps {
  /** True while the player (or a MultiNook tile) is full screen. */
  isPlayerFullscreen: () => boolean;
  /** Leaves the player's full screen (`exitPlayerFullscreen`). */
  exitPlayerFullscreen: () => void;
  /** Closes a badge detail that is already open. */
  closeBadgeDetail: () => void;
  /** Opens Badges on the Twitch tab with the link's badge pending. */
  openTwitchBadge: (setId: string, version: string) => void;
}

/** Read what `take_pending_badge_link` returned; null when it is not a link. */
export function readBadgeLink(raw: unknown): BadgeLinkTarget | null {
  if (!raw || typeof raw !== 'object') return null;
  const { setId, version } = raw as { setId?: unknown; version?: unknown };
  if (typeof setId !== 'string' || !setId) return null;
  return { setId, version: typeof version === 'string' && version ? version : '1' };
}

/**
 * Open the link's badge on desktop: leave full screen first (only when the
 * player is in it, so the overlays open in the window as they always do), close
 * a detail that is already open so the overlay can mount, then open Badges on
 * the Twitch tab.
 */
export function openBadgeLink(link: BadgeLinkTarget, deps: BadgeLinkDeps): void {
  if (deps.isPlayerFullscreen()) deps.exitPlayerFullscreen();
  deps.closeBadgeDetail();
  deps.openTwitchBadge(link.setId, link.version);
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
