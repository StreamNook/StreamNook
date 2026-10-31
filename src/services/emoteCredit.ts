// Who made an emote, for the hover cards in chat and the emote menu. Rust
// answers from the sets it already holds, then the provider's per-emote API,
// and caches per emote; this is the typed bridge and nothing more.
import { invoke } from '@tauri-apps/api/core';

export interface EmoteCredit {
  provider: 'twitch' | '7tv' | 'bttv' | 'ffz';
  /** The uploader (7TV, BTTV, FFZ) or the channel that owns it (Twitch). */
  ownerName?: string;
  /** The person who drew it, where the provider records one (Twitch, FFZ). */
  artistName?: string;
  /** The emote's own name when it is shown under an alias. */
  originalName?: string;
  /** "Sub emote", "Global emote", "Channel emote", ... */
  kindLabel?: string;
  /** Click card only: the people to credit, artist first. */
  creators?: EmoteCreator[];
  /** Click card only: the emote's own page on its provider (not Twitch). */
  pageUrl?: string;
  /** Click card only: Rust's line under the name ("Alias of x · 7TV channel emote"). */
  subtitle?: string;
  /** Click card only: the page button's words ("Open on 7TV"). */
  pageLabel?: string;
  /** Click card only, when the provider answered and credits nobody. */
  emptyLine?: string;
}

/** Where a creator chip goes. Rust builds every link from validated ids. */
export type EmoteCreatorLink =
  | { kind: 'twitch'; login: string; id?: string }
  | { kind: 'web'; url: string; site: string };

export interface EmoteCreator {
  /** `artist` drew it, `uploader` added it, `channel` owns a Twitch emote. */
  role: 'artist' | 'uploader' | 'channel';
  name: string;
  avatarUrl?: string;
  link: EmoteCreatorLink;
  /** Rust's label over the chip ("Art by", "Made by", "From the channel"). */
  roleLabel?: string;
  /** Rust's words for where the chip goes ("Twitch profile"). Absent on a creator built here. */
  destination?: string;
}

export interface EmoteCreditQuery {
  provider: string;
  id: string;
  /** The name it is shown under, so an alias can be told apart. */
  name?: string;
  channel?: string | null;
  channelId?: string | null;
  /** The click card: creators with pictures and links, and the emote's page. */
  detail?: boolean;
}

interface WireCredit {
  provider: EmoteCredit['provider'];
  owner_name?: string;
  artist_name?: string;
  original_name?: string;
  kind_label?: string;
  creators?: {
    role: EmoteCreator['role'];
    name: string;
    avatar_url?: string;
    link: EmoteCreatorLink;
    role_label?: string;
    destination?: string;
  }[];
  page_url?: string;
  subtitle?: string;
  page_label?: string;
  empty_line?: string;
}

/** The providers Rust can credit. Kick and YouTube emotes have no lookup. */
export function canCredit(provider: string): boolean {
  return provider === 'twitch' || provider === '7tv' || provider === 'bttv' || provider === 'ffz';
}

export async function fetchEmoteCredit(q: EmoteCreditQuery): Promise<EmoteCredit | null> {
  if (!canCredit(q.provider) || !q.id) return null;
  try {
    const c = await invoke<WireCredit | null>('emote_credit', {
      provider: q.provider,
      id: q.id,
      name: q.name ?? null,
      channel: q.channel || null,
      channelId: q.channelId || null,
      detail: q.detail ?? null,
    });
    if (!c) return null;
    return {
      provider: c.provider,
      ownerName: c.owner_name,
      artistName: c.artist_name,
      originalName: c.original_name,
      kindLabel: c.kind_label,
      creators: c.creators?.map((p) => ({
        role: p.role,
        name: p.name,
        avatarUrl: p.avatar_url,
        link: p.link,
        roleLabel: p.role_label,
        destination: p.destination,
      })),
      pageUrl: c.page_url,
      subtitle: c.subtitle,
      pageLabel: c.page_label,
      emptyLine: c.empty_line,
    };
  } catch {
    return null;
  }
}

const PROVIDER_LABEL: Record<string, string> = {
  twitch: 'Twitch',
  '7tv': '7TV',
  bttv: 'BetterTTV',
  ffz: 'FrankerFaceZ',
  kick: 'Kick',
  youtube: 'YouTube',
};

export function emoteProviderLabel(provider: string): string {
  return PROVIDER_LABEL[provider] ?? 'Emote';
}

/** The two lines for a credit, as plain strings. */
export function creditLines(provider: string, credit: EmoteCredit | null): [string, string] {
  const label = emoteProviderLabel(provider);
  if (!credit) return [label, ''];
  const kind = credit.kindLabel ?? '';
  const alias = credit.originalName ? `Alias of ${credit.originalName}` : '';
  if (provider === 'twitch') {
    // A Twitch emote's owner is the channel it belongs to; the artist, where
    // Twitch records one, is who drew it.
    if (credit.artistName) {
      return [`Art by ${credit.artistName}`, [credit.ownerName, kind || label].filter(Boolean).join(' · ')];
    }
    if (credit.ownerName) return [`From ${credit.ownerName}`, [label, kind].filter(Boolean).join(' · ')];
    return [label, kind];
  }
  if (credit.artistName) {
    const by = credit.ownerName && credit.ownerName !== credit.artistName ? `Added by ${credit.ownerName}` : '';
    return [`Art by ${credit.artistName}`, [alias || by, label].filter(Boolean).join(' · ')];
  }
  if (credit.ownerName) return [`by ${credit.ownerName}`, [alias || kind, label].filter(Boolean).join(' · ')];
  return [label, [alias, kind].filter(Boolean).join(' · ')];
}

/** The biggest image each provider serves for an emote, for the click card. */
export function emoteLargeUrl(provider: string, id: string, url: string): string {
  switch (provider) {
    case '7tv':
      return `https://cdn.7tv.app/emote/${id}/4x.avif`;
    case 'twitch':
      return url.replace(/\/[123]\.0$/, '/3.0');
    case 'bttv':
      return `https://cdn.betterttv.net/emote/${id}/3x`;
    case 'ffz':
      return url.replace(/\/[124]$/, '/4');
    default:
      return url;
  }
}
