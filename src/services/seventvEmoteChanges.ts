// The 7TV emote set changes a live update carries, as the stream overlay draws
// them. Pure, so it tests without the app's stores.

import type { Emote } from './emoteService';
import { seventvEmoteImageUrl, type SeventvEmoteChange } from '../components/overlay/overlayConfig';
import type { EmoteSetUpdatePayload } from './seventvEventApi';

/**
 * The payload's adds, removes and renames as the stream overlay draws them,
 * with each emote's 7TV image where it is known: an added emote from the
 * composed delta, a removed or renamed one from the set as it was before the
 * change (`before`), else from the composed removals' ids.
 */
export function seventvEmoteChanges(payload: EmoteSetUpdatePayload, before: readonly Emote[]): SeventvEmoteChange[] {
  const actor = payload.actor_name || 'Someone';
  const sevenTv = (e: { id: string; name: string; provider?: string }) => !e.provider || e.provider === '7tv';
  const addedRows = (payload.composed?.added ?? []).filter(sevenTv);
  const removedRows = payload.composed?.removed ?? [];
  const urlOf = (name: string, rows: readonly { id: string; name: string; url?: string; provider?: string }[]): string | undefined => {
    const row = rows.find((e) => e.name === name && sevenTv(e));
    if (!row) return undefined;
    return row.id ? seventvEmoteImageUrl(row.id) : row.url || undefined;
  };
  const out: SeventvEmoteChange[] = [];
  const push = (change: SeventvEmoteChange, url: string | undefined) => out.push(url ? { ...change, emoteUrl: url } : change);
  for (const name of payload.added) push({ action: 'added', actor, name }, urlOf(name, addedRows));
  for (const name of payload.removed) push({ action: 'removed', actor, name }, urlOf(name, before) ?? urlOf(name, removedRows));
  for (const r of payload.renamed) {
    push({ action: 'renamed', actor, name: r.new, oldName: r.old }, urlOf(r.new, addedRows) ?? urlOf(r.old, before));
  }
  return out;
}
