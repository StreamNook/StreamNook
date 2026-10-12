// Which chat channels the builder's live preview holds.
//
// A Twitch source is acquired only after its broadcaster id resolves (so the
// store can backfill history), which leaves a window where a source is wanted
// but not yet held. Dropping it in that window (switching to Sample, removing
// the source, leaving the builder) must neither release a reference this
// preview never took, which would take one some other view holds, nor acquire
// once the id lands, which would leak one for the rest of the session.

export interface SourceHoldOps {
  acquire(key: string, channelId: string | null): void;
  release(key: string): void;
}

export interface SourceHolds {
  /** Make `keys` the wanted set. New keys call `start(key)`, which acquires
   *  (directly or after an id resolves) through `take`; keys no longer wanted
   *  are released if they were taken. */
  want(keys: Iterable<string>, start: (key: string) => void): void;
  /** Acquire `key` now, if it is still wanted and not yet held. */
  take(key: string, channelId: string | null): void;
  /** Release everything held and want nothing (unmount). */
  dropAll(): void;
  readonly held: ReadonlySet<string>;
}

export function createSourceHolds(ops: SourceHoldOps): SourceHolds {
  const wanted = new Set<string>();
  const held = new Set<string>();
  return {
    held,
    want(keys, start) {
      const next = new Set(keys);
      for (const k of next) {
        if (wanted.has(k)) continue;
        wanted.add(k);
        start(k);
      }
      for (const k of Array.from(wanted)) {
        if (next.has(k)) continue;
        wanted.delete(k);
        if (held.delete(k)) ops.release(k);
      }
    },
    take(key, channelId) {
      if (!wanted.has(key) || held.has(key)) return;
      held.add(key);
      ops.acquire(key, channelId);
    },
    dropAll() {
      for (const k of Array.from(held)) ops.release(k);
      held.clear();
      wanted.clear();
    },
  };
}
