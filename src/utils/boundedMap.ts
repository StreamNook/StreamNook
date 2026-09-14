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
