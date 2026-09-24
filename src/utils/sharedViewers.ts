import type { Collaboration, TwitchStream } from '../types';
import { isTwitchStream } from './streamProvider';

/** Plain-language summary of a Shared Viewership group, for screen readers. */
export function collabLabel(collab: Collaboration): string {
  return `${collab.shared_viewers.toLocaleString()} watching together with ${collabNames(collab, Infinity)}`;
}

/** The whole group as a word: "both" for two, "all 3" from three up. */
export function groupWord(count: number): string {
  return count === 2 ? 'both' : `all ${count}`;
}

/** The others in the group as a phrase: "A", "A and B", "A, B and 3 more". */
export function collabNames(collab: Collaboration, max = 2): string {
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
