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
