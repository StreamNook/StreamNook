// Ranking for the composer's emote suggestions.
//
// DELIBERATE DUPLICATE of `getMatchingEmoteTokens` in
// `src/components/ChatWidget.tsx` (search for TAB_MATCH_LIMIT). Keep the two in
// step by hand; ranking rules change rarely, and the alternative was worse.
//
// The desktop version cannot be shared, and the reason is worth recording so
// nobody spends an afternoon rediscovering it. It is a `useCallback` inside the
// busiest component in the app, and two things in it resist being lifted out.
// It reads `ffzIsSubwoofer` from the store INSIDE the callback and deliberately
// outside its dependency list, so turning that into a parameter would freeze a
// live value at render time and quietly change when subscriber-only effects are
// offered. And its dependency list includes a function pulled non-reactively
// out of another store, so anything that alters its identity cascades into two
// more dependency lists in the same file. Neither risk is worth taking to a
// component the phone never renders.
//
// Ranking, unchanged from the desktop: dedupe by case-folded name so the
// higher-tier provider wins a collision, then order by provider tier, then
// favourites first, then alphabetically.
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { isFavoriteEmote } from '../../services/favoriteEmoteService';
import type { Emote, EmoteSet } from '../../services/emoteService';
import type { EmoteTabCandidate, EmoteTabMatchMode } from '../../utils/chatInputWord';

const TAB_MATCH_LIMIT = 50;

export interface EmoteMatchOptions {
  mode?: EmoteTabMatchMode;
  includeChatters?: boolean;
}

export function matchEmoteTokens(
  query: string,
  emotes: EmoteSet | null | undefined,
  options: EmoteMatchOptions = {},
): EmoteTabCandidate[] {
  if (!query) return [];
  const mode: EmoteTabMatchMode = options.mode ?? 'starts_with';
  const includeChatters = options.includeChatters ?? true;

  const q = query.toLowerCase();
  const seen = new Set<string>();
  type Ranked = { item: EmoteTabCandidate; providerTier: number; favoriteRank: number };
  const ranked: Ranked[] = [];

  const isAtQuery = q.startsWith('@');
  // A leading ':' is the Twitch-native trigger convention: ':Pog' matches the
  // emote 'Pog' (no emote name contains a colon) and floats Twitch emotes to
  // the front. A trailing ':' is tolerated. '@' and ':' are exclusive.
  const isColonQuery = q.startsWith(':');
  const stripAt = isAtQuery
    ? q.slice(1)
    : isColonQuery
      ? q.slice(1).replace(/:$/, '')
      : q;

  const test = (token: string) => {
    const t = token.toLowerCase();
    return mode === 'starts_with' ? t.startsWith(stripAt) : t.includes(stripAt);
  };

  if (emotes && !isAtQuery && stripAt) {
    // Walk providers in tier order so the seen-set drops cross-provider dupes
    // in favour of the higher tier. A colon query flips Twitch to the front.
    const ordered: Array<[Emote['provider'], Emote[] | undefined]> = isColonQuery
      ? [
          ['twitch', emotes.twitch],
          ['7tv', emotes['7tv']],
          ['bttv', emotes.bttv],
          ['ffz', emotes.ffz],
          ['kick', emotes.kick],
        ]
      : [
          ['7tv', emotes['7tv']],
          ['bttv', emotes.bttv],
          ['ffz', emotes.ffz],
          ['twitch', emotes.twitch],
          ['kick', emotes.kick],
        ];
    const tierOf = (provider: Emote['provider']) => ordered.findIndex(([p]) => p === provider);

    // Read once per call rather than per emote: this is the value the desktop
    // reads live inside its loop, and a suggestion list is built in one go, so
    // there is no window for it to change midway.
    const isSubwoofer = useAppStore.getState().ffzIsSubwoofer;

    for (const [provider, list] of ordered) {
      if (!list) continue;
      for (const e of list) {
        const key = e.name.toLowerCase();
        if (seen.has(key)) continue;
        if (!test(e.name)) continue;
        // Subscriber-only FFZ effects are not offered to non-subscribers. They
        // still render in incoming messages.
        if (e.ffzSubOnly && !isSubwoofer) continue;
        seen.add(key);
        ranked.push({
          providerTier: tierOf(provider),
          favoriteRank: isFavoriteEmote(e.id) ? 0 : 1,
          item: {
            name: e.name,
            priority: tierOf(provider),
            emote: {
              id: e.id,
              name: e.name,
              url: e.url,
              localUrl: e.localUrl,
              provider: e.provider,
              isZeroWidth: e.isZeroWidth,
              modifierFlags: e.modifierFlags,
              ffzSubOnly: e.ffzSubOnly,
            },
          },
        });
      }
    }
  }

  if (includeChatters && !isColonQuery) {
    const chatters = useChatUserStore.getState().getMatchingUsers(stripAt);
    for (const u of chatters) {
      const dn = u.displayName || u.username;
      const key = dn.toLowerCase();
      if (seen.has(key)) continue;
      if (!test(dn) && !test(u.username)) continue;
      seen.add(key);
      ranked.push({
        providerTier: 4, // chatters always after every emote provider
        favoriteRank: 1,
        item: {
          name: (isAtQuery ? '@' : '') + dn,
          priority: 4,
          chatter: { username: u.username, displayName: dn },
        },
      });
    }
  }

  ranked.sort((a, b) => {
    if (a.providerTier !== b.providerTier) return a.providerTier - b.providerTier;
    if (a.favoriteRank !== b.favoriteRank) return a.favoriteRank - b.favoriteRank;
    return a.item.name.localeCompare(b.item.name);
  });

  return ranked.slice(0, TAB_MATCH_LIMIT).map((r) => r.item);
}
