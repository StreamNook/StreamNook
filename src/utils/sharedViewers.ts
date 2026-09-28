import type { Collaboration, SharedChat, TwitchStream } from '../types';
import { isTwitchStream } from './streamProvider';

/** What a card credits beside a channel's name: a Shared Viewership group
 *  (Stream Together), or a Shared Chat session outside one. */
export type ChannelGroup = Collaboration | SharedChat;

/** A Shared Viewership group; only those carry a combined viewer count. */
export function isTogether(group: ChannelGroup): group is Collaboration {
  return 'shared_viewers' in group;
}

/** Plain-language summary of a group, for screen readers. */
export function collabLabel(collab: ChannelGroup): string {
  return isTogether(collab)
    ? `${collab.shared_viewers.toLocaleString()} watching together with ${collabNames(collab, Infinity)}`
    : `Sharing chat with ${collabNames(collab, Infinity)}`;
}

/** The whole group as a word: "both" for two, "all 3" from three up. */
export function groupWord(count: number): string {
  return count === 2 ? 'both' : `all ${count}`;
}

/** The others in the group as a phrase: "A", "A and B", "A, B and 3 more". */
export function collabNames(collab: ChannelGroup, max = 2): string {
  const others = collab.members.filter((m) => !m.is_self).map((m) => m.display_name);
  if (others.length <= max) {
    return others.length === 1 ? others[0] : `${others.slice(0, -1).join(', ')} and ${others[others.length - 1]}`;
  }
  return `${others.slice(0, max).join(', ')} and ${others.length - max} more`;
}

/** A card's group from the snapshot's map. The map is keyed by Twitch channel
 *  id, so any other platform's stream reads nothing rather than a stranger's. */
export function collabFor(
  collabs: Record<string, Collaboration>,
  stream: Pick<TwitchStream, 'provider' | 'user_id'>,
): Collaboration | undefined {
  return isTwitchStream(stream) && stream.user_id ? collabs[stream.user_id] : undefined;
}

/** What a card credits: its Shared Viewership group, else its Shared Chat
 *  session (Rust never reports both for one channel). */
export function groupFor(
  collabs: Record<string, Collaboration>,
  sharedChats: Record<string, SharedChat>,
  stream: Pick<TwitchStream, 'provider' | 'user_id'>,
): ChannelGroup | undefined {
  if (!isTwitchStream(stream) || !stream.user_id) return undefined;
  return collabs[stream.user_id] ?? sharedChats[stream.user_id];
}
