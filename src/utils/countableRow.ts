// Which live chat rows count as news (SN-40): the phone's cleared chat says how
// many arrived since it was put away, inside its chat button. A system line
// (Twitch's own notices) is not news, and neither is a row a setting hides from
// the list, which the list itself drops with the same rule (isHiddenEvent).
// The iPhone's twin is SNKit `ChatRoomBuffer.countsAsNew`.
import { isHiddenEvent, type EventLikeMessage } from './chatEvents';

/** The user id Twitch's own notices carry (chatConnectionStore's system rows). */
const SYSTEM_USER = 'tw-system';

function tagOf(tags: unknown, key: string): string | undefined {
  if (tags instanceof Map) return tags.get(key) as string | undefined;
  if (tags && typeof tags === 'object') return (tags as Record<string, string>)[key];
  return undefined;
}

/** A row the cleared chat counts: shown in the list and not a system line. */
export function countsAsNew(row: unknown, hidden: readonly string[] | undefined): boolean {
  if (typeof row === 'string') return row.length > 0;
  if (!row || typeof row !== 'object') return false;
  const r = row as EventLikeMessage & { tags?: unknown };
  if (tagOf(r.tags, 'user-id') === SYSTEM_USER) return false;
  return !isHiddenEvent(r, hidden);
}

/** How many of `rows` count. */
export function countNew(rows: readonly unknown[], hidden: readonly string[] | undefined): number {
  let n = 0;
  for (const row of rows) if (countsAsNew(row, hidden)) n += 1;
  return n;
}
