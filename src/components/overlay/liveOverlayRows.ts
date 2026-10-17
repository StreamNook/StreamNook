// Pure pieces of the builder's live preview feed (LiveOverlayFeed): which
// Twitch badge art the overlay draws, and which of the chat's system rows reach
// it. Kept apart from the feed so they test without the app's stores.

import type { BackendChatMessage } from '../../services/twitchChat';
import type { ProviderId } from '../../types/providers';
import type { SeventvEmoteChange } from './overlayConfig';
import type { OverlayMessage } from './sampleMessages';

type BadgeInfo = { localUrl?: string; url?: string; image_url_4x?: string; image_url_2x?: string; image_url_1x?: string };

/** The image a resolved Twitch badge draws with. */
export const twitchBadgeUrl = (info: BadgeInfo): string | undefined =>
  info.localUrl || info.image_url_4x || info.image_url_2x || info.image_url_1x || info.url;

/** The Twitch roles a channel can give its own FrankerFaceZ art. */
const FFZ_ROLE_SETS = new Set(['moderator', 'vip']);

/** A channel's FFZ moderator and VIP art (Rust `get_ffz_room_role_art`). */
export interface RoomRoleArt {
  moderator: string | null;
  vip: string | null;
}

/** Badge art from anywhere but Twitch's CDN is a channel's own (its FFZ art,
 *  which Rust puts in the channel's badge set while the chat setting is on). */
const isTwitchArt = (url: string | undefined): boolean => !url || url.startsWith('https://static-cdn.jtvnw.net/');

/**
 * A Twitch badge's image for the overlay. Moderator and VIP draw the channel's
 * FFZ art only when the overlay asks for it (`customRoleArt`); otherwise
 * Twitch's own art from the global set, even where the chat's channel set
 * carries the custom art.
 */
export function overlayTwitchBadgeUrl(
  setId: string,
  info: BadgeInfo,
  customRoleArt: RoomRoleArt | null | undefined,
  globalInfo: () => BadgeInfo | null | undefined,
): string | undefined {
  const own = twitchBadgeUrl(info);
  if (!FFZ_ROLE_SETS.has(setId)) return own;
  if (customRoleArt !== undefined) {
    const custom = setId === 'moderator' ? customRoleArt?.moderator : customRoleArt?.vip;
    return custom || own;
  }
  if (isTwitchArt(info.image_url_4x ?? info.url)) return own;
  const twitchOwn = globalInfo();
  return (twitchOwn && twitchBadgeUrl(twitchOwn)) || own;
}

type RowTags = Readonly<Record<string, string | undefined>> | undefined;

/**
 * The channel whose badge set a Twitch row's badges come from, as the chat
 * reads it: the channel a shared-chat row was sent in, else the row's own
 * channel, else the channel the feed knows the row belongs to. A row looked up
 * with no channel draws its subscriber badge from the global set, Twitch's
 * default star, instead of the channel's own art.
 */
export function badgeRoomOf(tags: RowTags, fallbackRoomId?: string | null): string | undefined {
  return tags?.['source-room-id'] || tags?.['room-id'] || fallbackRoomId || undefined;
}

/** `parseBadges` from the chat's badge cache: channel set first, then global. */
export type TwitchBadgeLookup = (
  badgeString: string,
  channelId?: string,
) => Array<{ key: string; info: (BadgeInfo & { title?: string }) | null }>;

/**
 * A Twitch row's badges as the overlay draws them, resolved against the row's
 * channel (see `badgeRoomOf`). Null when none resolve, so the caller keeps the
 * row's own badges.
 */
export function overlayTwitchBadges(
  raw: ReadonlyArray<{ name: string; version: string }>,
  tags: RowTags,
  lookup: TwitchBadgeLookup,
  roleArt: RoomRoleArt | null | undefined,
  fallbackRoomId?: string | null,
): Array<{ name: string; version: string; image_url_4x: string | undefined; title: string | undefined }> | null {
  if (raw.length === 0) return null;
  const room = badgeRoomOf(tags, fallbackRoomId);
  const resolved = lookup(raw.map((b) => `${b.name}/${b.version}`).join(','), room).flatMap((r) => {
    const info = r.info;
    if (!info) return [];
    const [name, version] = r.key.split('/');
    const url = overlayTwitchBadgeUrl(name, info, roleArt, () => lookup(r.key)[0]?.info);
    return [{ name, version, image_url_4x: url, title: info.title }];
  });
  return resolved.length > 0 ? resolved : null;
}

/** The overlay row for a 7TV emote set change the chat noted, or null for any
 *  other system row: those are this app's own notices (command replies,
 *  connection lines, room mode changes), which the OBS overlay never shows. */
export function systemRowForOverlay(m: BackendChatMessage, provider: ProviderId): OverlayMessage | null {
  const change = (m as BackendChatMessage & { seventvEmoteChange?: SeventvEmoteChange }).seventvEmoteChange;
  if (!change) return null;
  return {
    ...m,
    provider,
    username: '',
    display_name: '',
    badges: [],
    tags: {},
    seventvEmoteChange: change,
  };
}

/** Whether a stored chat row is a system row rather than a chatter's message. */
export const isSystemRow = (m: BackendChatMessage): boolean => {
  if (m.user_id === 'tw-system' || m.username === 'System') return true;
  const tags = m.tags as unknown;
  return tags instanceof Map && tags.get('user-id') === 'tw-system';
};
