// What a clicked or hovered badge is, for the hover card and the spotlight card.
//
// Every chat badge (Twitch, 7TV, the third-party chat clients, StreamNook, and
// Kick / YouTube / TikTok's own) becomes one `BadgeRef` here, so the hover card,
// the spotlight and the "Open in Badges" route read one shape. Presentation
// only: anything a provider feed knows beyond what the row already holds
// (descriptions, holder counts, ownership) comes from Rust `get_badge_detail`.

import { getBadgeImageUrls, getBadgeFallbackUrls } from '../services/seventvService';

/** Third-party chat clients whose badges Rust's feeds know (`get_badge_detail`). */
export const THIRD_PARTY_BADGE_PROVIDERS = [
  'ffz',
  'bttv',
  'chatterino',
  'homies',
  'moltorino',
  'chatsen',
  'chatty',
  'dankchat',
] as const;

/** Platforms that ship their own badge art in the message (no badge pages). */
export const PLATFORM_BADGE_PROVIDERS = ['kick', 'youtube', 'tiktok'] as const;

const PROVIDER_LABELS: Record<string, string> = {
  twitch: 'Twitch',
  '7tv': '7TV',
  ffz: 'FrankerFaceZ',
  bttv: 'BetterTTV',
  chatterino: 'Chatterino',
  homies: 'Homies',
  moltorino: 'Moltorino',
  chatsen: 'Chatsen',
  chatty: 'Chatty',
  dankchat: 'DankChat',
  streamnook: 'StreamNook',
  kick: 'Kick',
  youtube: 'YouTube',
  tiktok: 'TikTok',
};

/** A provider's name as people know it. Keys arrive in any case (`BTTV`). */
export function badgeProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider.toLowerCase()] ?? provider;
}

export function isThirdPartyBadgeProvider(provider: string): boolean {
  return (THIRD_PARTY_BADGE_PROVIDERS as readonly string[]).includes(provider.toLowerCase());
}

/** One image of a badge. `px` is the art's nominal edge in pixels when known
 *  (Twitch 18/36/72, 7TV 18/36/54/72); a single URL serving every size has none. */
export interface BadgeArtSource {
  url: string;
  px?: number;
}

/** The Twitch badge version BadgeDetailOverlay opens (BadgesOverlay's shape). */
export interface TwitchBadgeVersionRef {
  id: string;
  image_url_1x: string;
  image_url_2x: string;
  image_url_4x: string;
  title: string;
  description: string;
  click_action: string | null;
  click_url: string | null;
}

export interface BadgeRef {
  /** Lowercase provider key (`twitch`, `7tv`, `ffz`, `streamnook`, `kick`, ...). */
  provider: string;
  /** The provider's id for the badge, as the row holds it (Rust normalises). */
  id: string;
  title: string;
  description?: string;
  /** Smallest first. */
  art: BadgeArtSource[];
  /** Tried in order when the chosen art fails to load. */
  fallbackUrls?: string[];
  /** The badge's own link, when its provider gives one. */
  clickUrl?: string;
  /** Twitch only: the set and version the Badges overlay opens. */
  twitch?: { setId: string; version: TwitchBadgeVersionRef };
  /** StreamNook only: the cosmetic the member wears (absent for the plain mark). */
  cosmeticSlug?: string;
}

/** Pick the smallest image that is still sharp in a `boxPx` CSS-pixel box on
 *  this screen; the largest when none is big enough or sizes are unknown. */
export function pickBadgeArt(art: readonly BadgeArtSource[], boxPx: number, dpr = currentDpr()): string {
  if (art.length === 0) return '';
  const need = boxPx * dpr;
  const sharp = art.find((a) => a.px !== undefined && a.px >= need);
  return (sharp ?? art[art.length - 1]).url;
}

function currentDpr(): number {
  return typeof window !== 'undefined' && window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
}

/** Ascending art list from sized candidates, skipping blanks. A URL that serves
 *  several sizes (one file for 1x/2x/4x) loses its nominal size, since nobody
 *  knows which it really is. */
function artList(candidates: Array<{ url: string | null | undefined; px?: number }>): BadgeArtSource[] {
  const urls = candidates.filter((c): c is { url: string; px?: number } => !!c.url);
  const out: BadgeArtSource[] = [];
  for (const c of urls) {
    const repeated = urls.filter((o) => o.url === c.url).length > 1;
    if (out.some((o) => o.url === c.url)) continue;
    out.push(repeated ? { url: c.url } : { url: c.url, px: c.px });
  }
  // A repeated URL is the only copy of every size it stood for: keep it last
  // so the picker never prefers a smaller sized file over it by accident.
  return out.sort((a, b) => (a.px ?? Infinity) - (b.px ?? Infinity));
}

/** What a Twitch-row badge (twitchBadges.parseBadges, or a platform's own
 *  badge from twitchChat) carries. */
export interface TwitchRowBadgeInfo {
  id?: string;
  title?: string;
  description?: string;
  localUrl?: string;
  url?: string;
  image_url_1x?: string;
  image_url_2x?: string;
  image_url_4x?: string;
  click_action?: string | null;
  click_url?: string | null;
  /** Set on Kick / YouTube / TikTok badges; absent on Twitch's. */
  provider?: string;
}

/**
 * A badge from the Twitch slot of a chat row. Kick, YouTube and TikTok badges
 * ride the same slot but carry `provider`, and stay their platform's: they
 * never reach the Twitch badge page. `rowUrl` is the URL the row drew (a
 * cached copy of the 4x file), reused so the hover card draws it at once.
 */
export function twitchRowBadgeRef(key: string, info: TwitchRowBadgeInfo, rowUrl?: string): BadgeRef {
  const [setId = '', versionFromKey = ''] = key.split('/');
  const platform = info.provider?.toLowerCase();
  if (platform && platform !== 'twitch') {
    const url = info.localUrl || info.image_url_4x || info.image_url_1x || rowUrl || '';
    return {
      provider: platform,
      id: key,
      title: info.title || badgeProviderLabel(platform),
      art: artList([{ url }]),
    };
  }
  const versionId = info.id || versionFromKey;
  const title = info.title || setId;
  const x4 = info.image_url_4x ? rowUrl || info.image_url_4x : undefined;
  return {
    provider: 'twitch',
    id: `${setId}/${versionId}`,
    title,
    description: info.description || undefined,
    art: artList([
      { url: info.image_url_1x, px: 18 },
      { url: info.image_url_2x, px: 36 },
      { url: x4, px: 72 },
      ...(x4 || info.image_url_1x ? [] : [{ url: rowUrl || info.localUrl }]),
    ]),
    clickUrl: info.click_url || undefined,
    twitch: {
      setId,
      version: {
        id: versionId,
        image_url_1x: info.image_url_1x ?? '',
        image_url_2x: info.image_url_2x ?? '',
        image_url_4x: info.image_url_4x ?? '',
        title,
        description: info.description ?? '',
        click_action: info.click_action ?? null,
        click_url: info.click_url ?? null,
      },
    },
  };
}

/** A 7TV badge as the cosmetics API returns it. */
type SevenTvBadge = Parameters<typeof getBadgeImageUrls>[0];

/** A 7TV badge (the selected one chatUserStore holds). */
export function sevenTvBadgeRef(badge: SevenTvBadge): BadgeRef {
  const urls = getBadgeImageUrls(badge);
  return {
    provider: '7tv',
    id: badge.id,
    title: badge.description || badge.name || '7TV badge',
    art: artList([
      { url: urls.url1x, px: 18 },
      { url: urls.url2x, px: 36 },
      { url: urls.url3x, px: 54 },
      { url: urls.url4x, px: 72 },
    ]),
    fallbackUrls: badge.localUrl ? undefined : getBadgeFallbackUrls(badge.id),
  };
}

/** A third-party chat-client badge, from chat (`ThirdPartyBadge`) or the
 *  gallery (`ChatClientBadge`). */
export function thirdPartyBadgeRef(badge: {
  id: string;
  title: string;
  provider: string;
  description?: string;
  imageUrl?: string;
  image1x?: string;
  image2x?: string;
  image4x?: string;
  image_1x?: string;
  image_2x?: string;
  image_4x?: string;
  link?: string;
  click_url?: string | null;
}): BadgeRef {
  const provider = badge.provider.toLowerCase();
  return {
    provider,
    id: badge.id,
    title: badge.title || badgeProviderLabel(provider),
    description: badge.description || undefined,
    art: artList([
      { url: badge.image1x ?? badge.image_1x, px: 18 },
      { url: badge.image2x ?? badge.image_2x, px: 36 },
      { url: badge.image4x ?? badge.image_4x ?? badge.imageUrl, px: 72 },
    ]),
    clickUrl: badge.link || badge.click_url || undefined,
  };
}

/** The StreamNook badge. `artUrl` is the full-size asset (not the chat copy). */
export function streamNookBadgeRef(memberId: string, title: string, artUrl: string, cosmeticSlug?: string): BadgeRef {
  return { provider: 'streamnook', id: memberId, title, art: artList([{ url: artUrl }]), ...(cosmeticSlug ? { cosmeticSlug } : {}) };
}

/** Where "Open in Badges" goes for a badge; null for badges with no page. */
export type BadgeDetailsRoute =
  | { kind: 'twitch'; badge: TwitchBadgeVersionRef; setId: string }
  | { kind: '7tv'; badgeId: string }
  | { kind: 'streamnook'; slug?: string }
  | { kind: 'target'; tab: 'bttv' | 'chat-clients'; query: string };

export function badgeDetailsRoute(ref: BadgeRef): BadgeDetailsRoute | null {
  const provider = ref.provider.toLowerCase();
  if (provider === 'twitch') {
    return ref.twitch ? { kind: 'twitch', badge: ref.twitch.version, setId: ref.twitch.setId } : null;
  }
  if (provider === '7tv') return { kind: '7tv', badgeId: ref.id };
  if (provider === 'streamnook') return ref.cosmeticSlug ? { kind: 'streamnook', slug: ref.cosmeticSlug } : { kind: 'streamnook' };
  if (isThirdPartyBadgeProvider(provider)) {
    return { kind: 'target', tab: provider === 'bttv' ? 'bttv' : 'chat-clients', query: ref.title };
  }
  return null;
}

/** Open the Badges overlay on a badge (in main; from a popout it raises main).
 *  The router loads on the click, so this module stays light for chat rows. */
export async function openBadgeDetailsRoute(route: BadgeDetailsRoute): Promise<void> {
  const m = await import('./openBadgesInMain');
  switch (route.kind) {
    case 'twitch':
      m.openBadgeDetailInMain(route.badge, route.setId);
      return;
    case '7tv':
      m.openBadgesWithBadgeInMain(route.badgeId);
      return;
    case 'streamnook':
      m.openBadgesOnStreamNookInMain(route.slug);
      return;
    case 'target':
      m.openBadgesWithTargetInMain({ tab: route.tab, query: route.query });
  }
}

/** Badges with a full page of their own in the Badges overlay (Twitch, 7TV,
 *  StreamNook) open it straight from a click; the rest open the small card,
 *  which is the only place their details exist. Null when the card is right. */
export function directBadgeRoute(ref: BadgeRef): BadgeDetailsRoute | null {
  const route = badgeDetailsRoute(ref);
  return route && route.kind !== 'target' ? route : null;
}

/** Only https links leave the app from a badge card. */
export function safeBadgeLink(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).protocol === 'https:' ? url : null;
  } catch {
    return null;
  }
}
