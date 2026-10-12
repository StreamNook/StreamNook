import type { ProviderId } from '../../types/providers';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';

/**
 * The cosmetics id a MultiNook tile or preset channel's 7TV picture is looked
 * up by. `channelId` is the id the chat connection maps by: the user id on
 * Twitch, the channel's `UC...` id on YouTube. Kick is left out, because its
 * chat mapping id is not guaranteed to be the broadcaster's user id 7TV keys
 * Kick on.
 */
export function slotSevenTvId(slot: { provider?: ProviderId; channelId?: string }): string | null {
  const provider = slot.provider ?? 'twitch';
  if (provider === 'kick') return null;
  return sevenTvIdForStream({ provider, user_id: slot.channelId });
}
