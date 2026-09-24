// One time-ordered feed merged from several chat slices, whichever platforms
// they are on.
//
// Extracted from BlendedChatPane so the main chat panel can show the same merged
// feed without a second implementation. There must only ever be one: this merge
// has already been tuned once (it used to re-derive and re-sort the whole feed on
// every frame any source got a message, which pins the single webview main thread
// and visibly stutters a clip played over the feed), and a copy would drift from
// the tuned original the first time either side changed.
//
// The returned object's IDENTITY is stable on any tick that changed nothing, so
// memoized rows and the message list bail instead of re-rendering.

import { useMemo, useRef } from 'react';
import { useChatConnectionStore, sliceLookupKey, getActiveHistoryMax } from '../stores/chatConnectionStore';
import { parseKey } from '../utils/providerKey';
import { sourceKeyOf, sourceProviderOf, type ChatSource } from '../utils/sendToSource';
import type { BackendChatMessage } from '../services/twitchChat';
import type { ModerationContext } from './useTwitchChat';

/** The shape ChatWidget's message panel already consumes, plus the source map. */
export interface BlendedChatSource<T extends ChatSource> {
  messages: (string | BackendChatMessage)[];
  deletedMessageIds: Set<string>;
  clearedUserContexts: Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>;
  /** Summed per-source revision; doubles as the list's `renderToken`. */
  renderToken: number;
  /** messageId -> the source it came from. A raw Twitch IRC line does not carry
   *  its own slug, so this is the only way to route a reply back to the right
   *  channel. Handed back as a REF, not a value, so callbacks that read it can
   *  keep an empty dependency list and a fixed identity — which is what stops
   *  them defeating the row memo. */
  sourceRef: React.MutableRefObject<Map<string, T>>;
  /** Monotonic arrival counter, for the "N new since paused" badge. */
  seqRef: React.MutableRefObject<{ next: number }>;
}

/** Common sortable epoch-ms from either a structured message (`timestamp`, ISO-UTC
 *  for Kick and an epoch for Twitch) or a raw IRC string (`tmi-sent-ts`). */
export function tsOf(m: string | BackendChatMessage): number {
  const t = typeof m === 'string' ? m.match(/tmi-sent-ts=(\d+)/)?.[1] ?? '' : m.timestamp ?? '';
  if (!t) return 0;
  if (/^\d+$/.test(t)) return Number(t);
  const d = Date.parse(t);
  return Number.isNaN(d) ? 0 : d;
}

/** The most rows a merged feed will mount. The message list has no JavaScript
 *  windowing — `content-visibility` skips paint, not React reconciliation — and
 *  is comfortable to roughly a thousand rows, so this leaves generous headroom. */
const MERGED_ROW_CEILING = 400;

export function useBlendedChatSource<T extends ChatSource>(channels: T[]): BlendedChatSource<T> {
  // Re-render when any of THESE sources change. Summing the per-channel counters
  // (instead of the global revision) keeps the O(all sources) reconcile below from
  // re-running on flushes of channels this feed doesn't show.
  const renderToken = useChatConnectionStore((s) =>
    channels.reduce((sum, c) => {
      // Through the store's own derivation: bare login for Twitch, composite
      // otherwise, folded to the lowercase form storage actually uses.
      return sum + (s.revisionByChannel[sliceLookupKey(sourceProviderOf(c), c.channel)] ?? 0);
    }, 0),
  );

  // Incremental, append-only merge. The feed is ordered by FIRST-seen arrival and
  // only ever grows at the bottom (YouTube is polled, so a send-time sort would slot
  // its late messages mid-feed and thrash the scroll; freezing each slot on first
  // sight keeps everything on screen still). We keep a persistent ordered list of
  // ids and reconcile it against the live slices each tick: append genuinely-new ids
  // (send-time-sorted as one batch), refresh in-place upgrades (own-message repaint /
  // IRC echo / Helix-id stamp keep the SAME id but swap the slot reference), and drop
  // ids that have left every slice (per-channel cap eviction, or a source removed).
  // No per-frame re-sort.
  const orderRef = useRef<string[]>([]);
  const idToMsgRef = useRef<Map<string, string | BackendChatMessage>>(new Map());
  const idToChannelRef = useRef<Map<string, T>>(new Map());
  // Sources this feed has already taken a backlog decision about. A source that
  // joins mid-session arrives holding a full buffer of messages from minutes
  // ago; appending those (the merge orders by FIRST SIGHT, not send time) drops
  // them below the newest live row and scrolls to them, which reads as chat
  // jumping backwards. Its existing buffer is absorbed as already-seen instead,
  // so only what arrives after it joined is rendered.
  const seededRef = useRef<Set<string>>(new Set());
  // Rows dropped off the top by the cap below. The arrival counter has to keep
  // climbing past them, or "N new since paused" would shrink as the feed trims.
  const evictedRef = useRef(0);
  // Monotonic counter for the "N new since paused" badge. Order lives in orderRef,
  // so this drives only the unread count, not placement.
  const seqRef = useRef<{ next: number }>({ next: 0 });
  // Cached render outputs. Their references only change when their contents do, so a
  // quiet tick hands the memoized list the exact same props and it skips the work.
  const renderCacheRef = useRef<{
    messages: (string | BackendChatMessage)[];
    deleted: Set<string>;
    cleared: Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>;
  }>({ messages: [], deleted: new Set(), cleared: new Map() });

  const { messages, deletedMessageIds, clearedUserContexts } = useMemo(() => {
    const store = useChatConnectionStore.getState();
    // Match by the STORE MAP KEY via parseKey (the source of truth): a Kick slice
    // is keyed `kick:slug` and its own `.channel` field holds that composite key,
    // not the bare slug. parseKey normalizes both bare Twitch (`xqc`) and composite.
    const open = new Set(channels.map(sourceKeyOf));
    const byKey = new Map(channels.map((c) => [sourceKeyOf(c), c] as const));
    const keyOf = (m: string | BackendChatMessage) =>
      typeof m === 'string' ? m.match(/(?:^@|;)id=([^;]+)/)?.[1] ?? m : m.id;
    const order = orderRef.current;
    const idToMsg = idToMsgRef.current;
    const idToChannel = idToChannelRef.current;

    let structuralChange = false; // appended/removed -> messages identity must change
    let refChange = false; // an on-screen row's reference upgraded in place

    // One pass over the open slices: record presence, queue newcomers, refresh
    // in-place upgrades. Newcomers are gathered in store-iteration order so that
    // equal-timestamp ties resolve the same way the old full-rebuild did. This
    // reconcile is idempotent (driven off the slices, keyed by id), so React 18
    // StrictMode's double-invoke in dev is a harmless no-op the second time.
    const present = new Set<string>();
    const newcomers: Array<{ id: string; m: string | BackendChatMessage; ch?: T }> = [];
    // First tick for a source: swallow whatever it already holds. On the very
    // first tick of all (nothing rendered yet) there is no "live bottom" to
    // protect, so the opening backlog is kept and the feed starts populated.
    const opening = order.length === 0;
    for (const [key, slice] of store.channels.entries()) {
      const pk = parseKey(key);
      const skey = `${pk.provider}::${pk.channel.toLowerCase()}`;
      if (!open.has(skey) || seededRef.current.has(skey)) continue;
      seededRef.current.add(skey);
      if (opening) continue;
      for (const m of slice.messages) {
        const id = keyOf(m);
        if (id && !idToMsg.has(id)) idToMsg.set(id, m);
      }
    }
    // A source that left may come back; let it be absorbed again next time.
    for (const skey of Array.from(seededRef.current)) {
      if (!open.has(skey)) seededRef.current.delete(skey);
    }
    for (const [key, slice] of store.channels.entries()) {
      const pk = parseKey(key);
      const skey = `${pk.provider}::${pk.channel.toLowerCase()}`;
      if (!open.has(skey)) continue;
      const ch = byKey.get(skey);
      for (const m of slice.messages) {
        const id = keyOf(m);
        if (!id) continue; // unkeyable; can't track/dedupe/remove it reliably
        if (present.has(id)) continue; // a dup id across slices renders once (the list guards too)
        present.add(id);
        const prev = idToMsg.get(id);
        if (prev === undefined) {
          newcomers.push({ id, m, ch });
        } else if (prev !== m) {
          // Same id, new slot reference: an in-place upgrade (own repaint / echo /
          // Helix-id stamp). Refresh the held reference so the row repaints.
          idToMsg.set(id, m);
          refChange = true;
          if (ch) {
            idToChannel.set(id, ch);
            const tagId = typeof m !== 'string' ? m.tags?.['id'] : undefined;
            if (tagId && tagId !== id) idToChannel.set(tagId, ch);
          }
        }
      }
    }

    // Removals: ids we hold that no longer appear in any open slice (cap eviction,
    // or the source was removed from the feed). `present` already includes this
    // tick's newcomers (still absent from idToMsg), so a size comparison can't tell
    // us anything, so scan the held ids directly. Compact the order array in one pass
    // only when something was actually dropped.
    let removedAny = false;
    for (const id of Array.from(idToMsg.keys())) {
      if (!present.has(id)) {
        idToMsg.delete(id);
        idToChannel.delete(id);
        removedAny = true;
      }
    }
    if (removedAny) {
      let w = 0;
      for (let r = 0; r < order.length; r++) {
        if (idToMsg.has(order[r])) order[w++] = order[r];
      }
      order.length = w;
      structuralChange = true;
    }

    // Append newcomers as one send-time-ordered batch (Array.sort is stable, so
    // equal timestamps keep their store-iteration order).
    if (newcomers.length) {
      newcomers.sort((a, b) => tsOf(a.m) - tsOf(b.m));
      for (const x of newcomers) {
        idToMsg.set(x.id, x.m);
        order.push(x.id);
        if (x.ch) {
          idToChannel.set(x.id, x.ch);
          const tagId = typeof x.m !== 'string' ? x.m.tags?.['id'] : undefined;
          if (tagId && tagId !== x.id) idToChannel.set(tagId, x.ch);
        }
      }
      structuralChange = true;
    }

    // Cap the COMBINED feed. Every source caps its own buffer, so three of them
    // at a raised `message_buffer_cap` would mount thousands of rows here — and
    // the message list virtualizes with `content-visibility`, which skips paint
    // but not React reconciliation.
    //
    // Not a flat per-source cap though: applying 100 to a two-platform feed
    // halves everyone's scrollback, which is a real loss for the reader. It
    // scales with the number of sources and stops at a ceiling the unwindowed
    // list is comfortable with, so scrollback grows with the feed while the DOM
    // stays bounded whatever the buffer setting is. Trim from the top, where
    // the oldest are.
    const cap = Math.min(getActiveHistoryMax() * Math.max(1, channels.length), MERGED_ROW_CEILING);
    if (order.length > cap) {
      const drop = order.length - cap;
      for (let i = 0; i < drop; i++) {
        const id = order[i];
        idToMsg.delete(id);
        idToChannel.delete(id);
      }
      order.splice(0, drop);
      evictedRef.current += drop;
      structuralChange = true;
    }

    // The arrival counter is derived from what the order array actually holds
    // rather than incremented per newcomer. Incrementing inside the memo is a
    // render-phase mutation, so StrictMode's double-invoke counted every message
    // twice in dev even though the reconcile itself is idempotent.
    seqRef.current.next = order.length + evictedRef.current;

    // Moderation sets are FLAGGED on the slice (never spliced from messages), so
    // they stay cheap to re-collect; reuse the prior reference when unchanged so a
    // quiet tick doesn't re-render the list.
    const cache = renderCacheRef.current;
    const deleted = new Set<string>();
    const cleared = new Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>();
    for (const [key, slice] of store.channels.entries()) {
      const pk = parseKey(key);
      const skey = `${pk.provider}::${pk.channel.toLowerCase()}`;
      if (!open.has(skey)) continue;
      slice.deletedMessageIds?.forEach((id: string) => deleted.add(id));
      slice.clearedUserContexts?.forEach(
        (v: { context: ModerationContext; affectedMessageIds: Set<string> }, k: string) => cleared.set(k, v),
      );
    }
    const deletedSame =
      deleted.size === cache.deleted.size && [...deleted].every((id) => cache.deleted.has(id));
    const clearedSame =
      cleared.size === cache.cleared.size && [...cleared.keys()].every((k) => cache.cleared.has(k));
    if (!deletedSame) cache.deleted = deleted;
    if (!clearedSame) cache.cleared = cleared;

    // Rebuild the rendered array only when the set or a reference actually changed;
    // otherwise hand back the identical reference so the memoized rows + list bail.
    if (structuralChange || refChange || cache.messages.length !== order.length) {
      cache.messages = order.map((id) => idToMsg.get(id) as string | BackendChatMessage);
    }
    return {
      messages: cache.messages,
      deletedMessageIds: cache.deleted,
      clearedUserContexts: cache.cleared,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channels, renderToken]);

  return {
    messages,
    deletedMessageIds,
    clearedUserContexts,
    renderToken,
    sourceRef: idToChannelRef,
    seqRef,
  };
}
