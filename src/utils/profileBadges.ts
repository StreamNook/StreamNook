// Normalise a user's badges from every source the app decorates profiles with,
// into one shape both shells can render.
//
// Lifted verbatim from the useMemo inside UserProfileCard (desktop), which is
// where this logic has always lived as a local closure. Reimplementing it for
// mobile would have meant two copies of the merge/dedupe rules drifting apart
// the first time a provider changed its payload, and these rules are fiddlier
// than they look: each source spells its image fields differently, the cache
// and the Rust profile can both carry the same badge, and Twitch splits the
// same badge across display and earned lists.
//
// The desktop card still has its own copy; migrating it to import this is
// tracked separately, since editing that component is a large change with real
// regression surface.
import { getBadgeImageUrls, getBadgeFallbackUrls } from '../services/seventvService';

export interface NormalizedBadge {
  id: string;
  src?: string;
  srcSet?: string;
  fallbackUrls?: string[];
  title?: string;
  name?: string;
  description?: string;
  provider?: string;
}

export interface NormalizedProfileBadges {
  twitch: NormalizedBadge[];
  seventv: NormalizedBadge[];
  thirdParty: NormalizedBadge[];
  total: number;
}

interface Input {
  /** From cosmeticsCache.getFullProfileWithFallback. */
  cachedProfile?: { twitchBadges?: unknown[]; seventvCosmetics?: { badges?: unknown[] }; thirdPartyBadges?: unknown[] } | null;
  /** The Rust `get_user_profile` payload, when a caller has one. */
  profileData?: Record<string, unknown> | null;
  /** Resolved over the BTTV socket, not the cached contributor feed. */
  bttvProBadge?: { url?: string } | null;
}

/** Keeps the FIRST occurrence, so cached entries win over freshly fetched ones. */
function dedupe(lists: unknown[][], key: (b: never) => string): unknown[] {
  const map = new Map<string, unknown>();
  for (const list of lists) {
    for (const b of list) {
      const k = key(b as never);
      if (!map.has(k)) map.set(k, b);
    }
  }
  return Array.from(map.values());
}

export function normalizeProfileBadges({
  cachedProfile,
  profileData,
  bttvProBadge,
}: Input): NormalizedProfileBadges {
  const pd = (profileData ?? {}) as Record<string, never>;
  const badgesNode = (pd.badges ?? {}) as Record<string, unknown[]>;

  // Twitch splits the same badge across display and earned lists, so both are
  // merged before deduping.
  const twitchRaw = dedupe(
    [
      (cachedProfile?.twitchBadges ?? []) as unknown[],
      (badgesNode.display_badges ?? []) as unknown[],
      (badgesNode.earned_badges ?? []) as unknown[],
    ],
    (b: never) => {
      const x = b as Record<string, unknown>;
      return String(x.id ?? x.setID);
    },
  );
  const twitch: NormalizedBadge[] = twitchRaw.map((raw) => {
    const b = raw as Record<string, string>;
    return {
      id: b.id || `${b.setID}-${b.version}`,
      src: b.image4x || b.image_4x || b.image1x || b.image_1x,
      srcSet: `${b.image1x || b.image_1x} 1x, ${b.image2x || b.image_2x} 2x, ${b.image4x || b.image_4x} 4x`,
      title: b.title,
      description: b.description,
    };
  });

  const seventvRaw = dedupe(
    [
      (cachedProfile?.seventvCosmetics?.badges ?? []) as unknown[],
      ((pd.seventv_cosmetics as Record<string, unknown[]> | undefined)?.badges ?? []) as unknown[],
    ],
    (b: never) => String((b as Record<string, unknown>).id),
  );
  const seventv: NormalizedBadge[] = seventvRaw.map((raw) => {
    const b = raw as Record<string, string>;
    const urls = getBadgeImageUrls(b as never);
    return {
      id: b.id,
      src: urls.url4x || `https://cdn.7tv.app/badge/${b.id}/4x.webp`,
      fallbackUrls: getBadgeFallbackUrls(b.id).slice(1),
      srcSet: urls.url1x ? `${urls.url1x} 1x, ${urls.url2x} 2x, ${urls.url4x} 4x` : undefined,
      title: b.tooltip || b.description || b.name,
      name: b.name,
    };
  });

  const thirdPartyRaw = dedupe(
    [
      (cachedProfile?.thirdPartyBadges ?? []) as unknown[],
      (badgesNode.third_party_badges ?? []) as unknown[],
    ],
    (b: never) => String((b as Record<string, unknown>).id),
  );
  const thirdParty: NormalizedBadge[] = thirdPartyRaw.map((raw) => {
    const b = raw as Record<string, string>;
    return {
      id: b.id,
      src: b.image4x || b.imageUrl,
      srcSet:
        b.image1x && b.image2x && b.image4x
          ? `${b.image1x} 1x, ${b.image2x} 2x, ${b.image4x} 4x`
          : undefined,
      title: b.title,
      provider: b.provider,
    };
  });

  // Tagged BTTV so it groups with any contributor badge from the same provider.
  if (bttvProBadge?.url) {
    thirdParty.push({ id: 'bttv-pro', src: bttvProBadge.url, title: 'BTTV Pro', provider: 'BTTV' });
  }

  return {
    twitch,
    seventv,
    thirdParty,
    total: twitch.length + seventv.length + thirdParty.length,
  };
}
