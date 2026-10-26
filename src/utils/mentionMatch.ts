// The composer's @ list: chatters whose login, shown name or (user-set)
// nickname starts with what is typed, newest first, with the room's streamer
// leading whenever they match. Runs per keystroke, so it stays synchronous.
import type { ChatUser } from '../stores/chatUserStore';

const MATCH_SCAN_CAP = 200;

// Lowercased name pairs, keyed by user-object identity. Identity changes
// (rename, color) replace the ChatUser object via the identity coalescer, so a
// stale entry can never be served; the in-place lastSeen bump never touches
// names. Kills the 2-per-user toLowerCase() allocations the autocomplete scan
// paid on every keystroke.
const lowerNameCache = new WeakMap<ChatUser, { u: string; d: string }>();
function lowerNames(user: ChatUser): { u: string; d: string } {
  let l = lowerNameCache.get(user);
  if (!l) {
    l = { u: user.username.toLowerCase(), d: user.displayName.toLowerCase() };
    lowerNameCache.set(user, l);
  }
  return l;
}

export function matchingUsers(
  users: ReadonlyMap<string, ChatUser>,
  query: string,
  limit: number,
  lead: ChatUser | null,
  overrides: Readonly<Record<string, { nickname?: string | null }>>,
): ChatUser[] {
  const queryLower = query.toLowerCase();

  // Inserting an @mention still uses user.username (the real Twitch login)
  // because Twitch IRC doesn't resolve nicknames.
  const matchesQuery = (user: ChatUser): boolean => {
    const nick = overrides[user.userId]?.nickname?.toLowerCase();
    const lower = lowerNames(user);
    return lower.u.startsWith(queryLower) || lower.d.startsWith(queryLower) || !!(nick && nick.startsWith(queryLower));
  };

  // The streamer leads, wearing their own row's color and paint once they
  // have chatted here.
  const owner = lead && matchesQuery(lead) ? (users.get(lead.userId) ?? lead) : null;
  const ownerLogin = owner?.username.toLowerCase();

  const matches: ChatUser[] = [];
  for (const user of users.values()) {
    if (owner && (user === owner || user.username.toLowerCase() === ownerLogin)) continue;
    if (matchesQuery(user)) {
      matches.push(user);
      // Runs per keystroke over up to ~9k users. Cap the collect: past this
      // many matches (single-letter queries), the top-`limit` by recency is
      // close enough for autocomplete and the full sort isn't worth it.
      if (matches.length >= MATCH_SCAN_CAP) break;
    }
  }

  // Sort by recency (most recent first)
  matches.sort((a, b) => b.lastSeen - a.lastSeen);

  return owner ? [owner, ...matches.slice(0, limit - 1)] : matches.slice(0, limit);
}
