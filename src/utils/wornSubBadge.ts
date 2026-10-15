// The sub badge a chatter wears in a room, read from the badges chat hands a
// profile when it opens one.

/** A badge as chat hands it to the profile: `subscriber/3012` plus its art. */
export interface WornBadge {
  key: string;
  info?: Record<string, unknown> | null;
}

/** The sub or founder badge art a chatter wears in this room, if any. */
export function wornSubBadgeArt(badges: readonly WornBadge[] | undefined): string | null {
  const info = (badges ?? []).find((b) => /^(founder|subscriber)\//.test(String(b.key ?? '')))?.info;
  if (!info) return null;
  const url = [info.image_url_4x, info.image_url_2x, info.image_url_1x, info.localUrl].find(
    (u): u is string => typeof u === 'string' && u.length > 0,
  );
  return url ?? null;
}
