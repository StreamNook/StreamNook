/**
 * Insert into a module-level cache Map with a hard size cap. When the map is
 * at capacity and the key is new, the oldest-inserted entry is evicted first
 * (Map iteration order is insertion order). Keeps session-long caches from
 * growing without bound; a rare re-fetch after eviction is the accepted cost.
 */
export function boundedSet<K, V>(map: Map<K, V>, key: K, value: V, max: number): void {
  if (!map.has(key) && map.size >= max) {
    const oldest = map.keys().next();
    if (!oldest.done) map.delete(oldest.value);
  }
  map.set(key, value);
}

/**
 * Add to a module-level Set with a hard size cap, evicting the oldest-inserted
 * value first when the set is full and the value is new (Set iteration order is
 * insertion order). Same eviction order as boundedSet.
 */
export function boundedAdd<V>(set: Set<V>, value: V, max: number): void {
  if (!set.has(value) && set.size >= max) {
    const oldest = set.values().next();
    if (!oldest.done) set.delete(oldest.value);
  }
  set.add(value);
}
