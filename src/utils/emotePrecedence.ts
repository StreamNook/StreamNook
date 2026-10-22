// Which emote a name means when several sets define it, the order Rust's chat
// parser draws them in (`EmoteLookup::build` in irc_service.rs): Twitch's own
// emotes first (a name Twitch would tag is Twitch's emote), then a channel's
// own third-party emotes before any global of the same name, and inside each
// group 7TV > FFZ > BetterTTV (the 7TV extension's order). Kick and YouTube
// come last; their names rarely meet the others.

import type { Emote, EmoteSet } from '../services/emoteService';

/** The `emote_type` Rust stamps on a third-party global emote. */
const GLOBAL_EMOTE_TYPE = 'global';

const THIRD_PARTY = ['7tv', 'ffz', 'bttv'] as const;

/** The set's emote lists, the one that wins a shared name first. Kick's and
 *  YouTube's own emotes are left out with `withPlatformEmotes: false`. */
export function listsByPrecedence(set: EmoteSet, { withPlatformEmotes = true } = {}): Emote[][] {
  const isGlobal = (e: Emote) => e.emote_type === GLOBAL_EMOTE_TYPE;
  return [
    set.twitch ?? [],
    ...THIRD_PARTY.map((p) => (set[p] ?? []).filter((e) => !isGlobal(e))),
    ...THIRD_PARTY.map((p) => (set[p] ?? []).filter(isGlobal)),
    ...(withPlatformEmotes ? [set.kick ?? [], set.youtube ?? []] : []),
  ];
}
