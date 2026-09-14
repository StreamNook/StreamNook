// Why you might not be able to talk in this room.
//
// Two different kinds of claim here, and they are kept apart deliberately:
//
//   `labels`  - what the room has switched ON. Purely factual, straight off
//               ROOMSTATE, so it is always safe to show.
//   `blocked` - a claim that YOU specifically cannot send. Only ever asserted
//               from something authoritative, never a guess: your own badges for
//               the channel you are in, or an explicit follow check.
//
// Follower-only used to be excluded here, on the grounds that nothing in the
// chat payload says whether you follow and guessing wrong either hides a working
// composer or promises one that silently drops messages. That reasoning was
// right; the conclusion was too narrow. The answer simply is not in the chat
// payload - it is a Helix call (`useFollowStatus`), and with a real answer in
// hand there is no guess to get wrong. Leaving it out meant an unfollowed viewer
// got an inviting "Send a message", typed one, and only then learned the room
// was closed to them.
import type { RoomState } from '../../stores/chatConnectionStore';

export interface ChatGating {
  /** Active restrictions, in the order they should read. Empty when unrestricted. */
  labels: string[];
  /** Set only when badges or an explicit follow check prove you cannot send. */
  blocked: 'subs' | 'emote' | 'follow' | null;
  /** One-line explanation for the composer when blocked. */
  reason: string | null;
}

function hasAny(badges: string | null | undefined, names: string[]): boolean {
  if (!badges) return false;
  return names.some((n) => badges.includes(n));
}

/** Elevated roles bypass every chat restriction Twitch applies. */
const ELEVATED = ['broadcaster', 'moderator', 'vip'];

export function deriveChatGating(
  room: RoomState | null | undefined,
  userBadges: string | null | undefined,
  /** From `useFollowStatus`. `null` means not known yet, or the check failed,
   *  and null must never block - see the follower-only branch below. */
  isFollowing: boolean | null = null,
): ChatGating {
  if (!room) return { labels: [], blocked: null, reason: null };

  const labels: string[] = [];
  if (room.subsOnly) labels.push('Subscribers only');
  if (room.followersOnly === 0) labels.push('Followers only');
  else if (room.followersOnly > 0) labels.push(`Followers only (${room.followersOnly}m)`);
  if (room.emoteOnly) labels.push('Emote only');
  if (room.slow > 0) labels.push(`Slow ${room.slow}s`);
  if (room.r9k) labels.push('Unique messages only');

  const elevated = hasAny(userBadges, ELEVATED);
  let blocked: ChatGating['blocked'] = null;
  let reason: string | null = null;

  if (!elevated) {
    if (room.subsOnly && !hasAny(userBadges, ['subscriber', 'founder'])) {
      blocked = 'subs';
      reason = 'Subscribers only';
    } else if (room.emoteOnly) {
      // Emote-only blocks free text for everyone unelevated, whether or not you
      // subscribe, so this one needs no badge check beyond the bypass.
      blocked = 'emote';
      reason = 'Emote only. Send emotes from the picker.';
    } else if (
      // -1 is off; 0 is any follower; >0 is "followed for N minutes".
      room.followersOnly >= 0 &&
      // Only `false` blocks. `null` is "we do not know", and an unknown must
      // leave the composer working: a message that gets rejected is recoverable,
      // a composer locked for no reason gives the user nothing to act on.
      isFollowing === false &&
      // Subscribing bypasses follower mode, and a subscriber who somehow does
      // not follow would otherwise be told to follow for no reason.
      !hasAny(userBadges, ['subscriber', 'founder'])
    ) {
      // Note this deliberately does NOT try to enforce the >0 minute variant for
      // someone who DOES follow: the follow check answers whether, not since
      // when. Not following at all is blocking either way, which is the case
      // that actually bites; a brand-new follower inside the waiting window
      // keeps a working composer and one possible rejection.
      blocked = 'follow';
      reason = 'Follow to send a message';
    }
  }

  return { labels, blocked, reason };
}
