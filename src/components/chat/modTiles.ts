// The drag-to-moderate tiles and how one is picked, shared by the chat's drag
// layer and the settings preview so both look and pick exactly the same.

import { ArrowCounterClockwise, Clock, Prohibit, PushPin, PushPinSlash, Trash } from 'phosphor-react';

import type { Bucket } from './ModBucketTile';

export const TIMEOUT_TILE: Bucket = {
  id: 'timeout',
  label: 'Timeout',
  icon: Clock,
  kind: 'danger',
  activeTint: 'bg-amber-500/25 border-amber-400/70 text-amber-200',
};

export const BAN_TILE: Bucket = {
  id: 'ban',
  label: 'Ban',
  icon: Prohibit,
  kind: 'danger',
  activeTint: 'bg-red-500/25 border-red-400/70 text-red-200',
};

export const DELETE_TILE: Bucket = {
  id: 'delete',
  label: 'Delete',
  icon: Trash,
  kind: 'danger',
  activeTint: 'bg-orange-500/25 border-orange-400/70 text-orange-200',
};

export const PIN_TILE: Bucket = {
  id: 'pin',
  label: 'Pin',
  icon: PushPin,
  kind: 'neutral',
  activeTint: 'bg-sky-500/25 border-sky-400/70 text-sky-200',
};

export const UNPIN_TILE: Bucket = { ...PIN_TILE, id: 'unpin', label: 'Unpin', icon: PushPinSlash };

/** Lifts a timeout (Untimeout) or a ban (Unban). */
export const liftTile = (timedOut: boolean): Bucket => ({
  id: 'unban',
  label: timedOut ? 'Untimeout' : 'Unban',
  icon: ArrowCounterClockwise,
  kind: 'danger',
  activeTint: 'bg-emerald-500/25 border-emerald-400/70 text-emerald-200',
});

/** Extra room before Ban beside chat, so a move that overshoots Timeout
 *  lands in empty space, never on Ban. */
export const BAN_GAP_PX = 24;

/** A tile counts as picked only while the cursor is on it, plus this much:
 *  half the column's 10 px gap, so the gap is split between its two tiles
 *  instead of handed to one of them. */
const HIT_MARGIN_PX = 5;

/**
 * The tile under the pointer, or null. The picked tile keeps priority until
 * the pointer leaves its own box (it scales up, so it does not flicker at the
 * edge); otherwise the nearer center wins, never whichever tile comes last in
 * the page.
 */
export function pickTile(tiles: Iterable<HTMLElement>, x: number, y: number, armed: string | null): string | null {
  let picked: string | null = null;
  let nearest = Infinity;
  for (const tile of tiles) {
    const r = tile.getBoundingClientRect();
    if (x < r.left - HIT_MARGIN_PX || x > r.right + HIT_MARGIN_PX || y < r.top - HIT_MARGIN_PX || y > r.bottom + HIT_MARGIN_PX) {
      continue;
    }
    const id = tile.getAttribute('data-bucket-id');
    const distance = id === armed ? -1 : Math.hypot(x - (r.left + r.width / 2), y - (r.top + r.height / 2));
    if (distance < nearest) {
      nearest = distance;
      picked = id;
    }
  }
  return picked;
}
