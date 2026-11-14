// Which wall tile a badge link names. A tile merges badges by title, so it
// carries every `set/version` key it covers (`tile.keys`); a link's key
// (`<set>/<version>`) matches when ANY of them does: the set id compared
// case-insensitively (links lowercase it), the version exactly.

interface TileKeys {
  keys: string[];
}

function splitKey(key: string): [string, string] {
  const at = key.indexOf('/');
  return at < 0 ? [key, ''] : [key.slice(0, at), key.slice(at + 1)];
}

/**
 * What a link's read answer does: nothing (the wall is not there yet, so the
 * Badges tab shows its own state), open the tile's sheet, or say it is missing.
 */
export function badgeLinkOutcome<T extends TileKeys>(
  wall: { loading: boolean; tiles: readonly T[] },
  key: string,
): { kind: 'none' } | { kind: 'open'; tile: T } | { kind: 'miss' } {
  if (wall.loading) return { kind: 'none' };
  const tile = tileForKey(wall.tiles, key);
  return tile ? { kind: 'open', tile } : { kind: 'miss' };
}

export function tileForKey<T extends TileKeys>(tiles: readonly T[], key: string): T | undefined {
  const [setId, version] = splitKey(key);
  const wantedSet = setId.toLowerCase();
  return tiles.find((tile) =>
    tile.keys.some((k) => {
      const [kSet, kVersion] = splitKey(k);
      return kSet.toLowerCase() === wantedSet && kVersion === version;
    }),
  );
}
