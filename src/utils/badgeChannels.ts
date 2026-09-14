// Finds the Twitch channels a badge's earn text points at, so the More Info
// panel can render them as clickable chips and cards.
//
// Badge copy names a channel in several ways and rarely as a bare login:
// "/studbudz", "twitch.tv/fps_shaka", "Ibai's channel", "the participating
// channel StudBudz". Scraped copy also uses the curly apostrophe, so the
// possessive form must accept both.
//
// Callers resolve every candidate through `search_channels` and drop the ones
// that do not exist, so this errs toward offering candidates rather than
// filtering hard here.

// Words that follow "channel" or sit in a possessive without naming a streamer.
const NOT_A_CHANNEL = new Set([
  'twitch', 'the', 'this', 'that', 'these', 'those', 'any', 'all', 'each', 'one', 'both',
  'a', 'an', 'and', 'or', 'to', 'in', 'on', 'at', 'for', 'from', 'with', 'via', 'by',
  'you', 'your', 'their', 'his', 'her', 'its', 'our', 'my',
  'participating', 'eligible', 'partner', 'partnered', 'affiliate', 'selected', 'select',
  'specific', 'certain', 'multiple', 'single', 'other', 'another', 'following', 'listed',
  'live', 'stream', 'streams', 'streamer', 'streamers', 'channel', 'channels', 'category',
  'categories', 'directory', 'event', 'events', 'badge', 'badges', 'campaign', 'drops', 'drop',
  'during', 'while', 'when', 'must', 'need', 'watch', 'watching', 'subscribe', 'subscription',
  'gift', 'gifted', 'gifting', 'viewers', 'viewer', 'users', 'user', 'source', 'note', 'notes',
  'important', 'however', 'also', 'prime', 'turbo', 'access', 'games', 'game', 'special',
  'official', 'main', 'new', 'first', 'second', 'third', 'games',
]);

// Twitch logins: 4 to 25 characters, letters/digits/underscore, not starting with a digit.
const LOGIN = /^[a-zA-Z][a-zA-Z0-9_]{3,24}$/;

// Site routes that follow twitch.tv/ without naming a channel.
const SITE_ROUTE = /^(directory|videos|settings|drops|turbo|prime|subscriptions|downloads|jobs|about|legal|help|store)$/i;

/** Whether a name written after a slash can be a channel login at all. */
export function isChannelMention(name: string): boolean {
  return LOGIN.test(name) && !NOT_A_CHANNEL.has(name.toLowerCase()) && !SITE_ROUTE.test(name);
}

// A mention as badge copy writes one: "/studbudz", "twitch.tv/studbudz",
// "https://www.twitch.tv/studbudz". The host part is optional, which is also
// what keeps subdomain links (help.twitch.tv/...) out: their slash sits right
// after a "." and the boundary check below rejects it.
const MENTION = /(?:https?:\/\/)?(?:www\.)?(?:twitch\.tv)?\/([a-zA-Z_][a-zA-Z0-9_]{3,24})\b/g;

export interface BadgeTextPart {
  /** The text exactly as written, so a non-mention run renders unchanged. */
  text: string;
  /** Set when this run is a channel mention. Lowercased login. */
  login?: string;
}

/**
 * Badge copy split into plain runs and the channel mentions inside it, so the
 * More Info panel can chip the mentions and leave everything else as prose.
 *
 * The boundary rule is the whole job. A slash inside a word or a URL path is
 * just a slash: without that check "Pokémon Scarlet/Violet" chipped "Violet"
 * and "help.twitch.tv/s/article/pokemon-chat-badges" chipped "article" and
 * "pokemon", each one a link to a channel that mostly does not exist.
 */
export function splitChannelMentions(text: string): BadgeTextPart[] {
  const parts: BadgeTextPart[] = [];
  if (!text) return parts;

  let cut = 0;
  for (const m of text.matchAll(MENTION)) {
    const start = m.index ?? 0;
    const before = start > 0 ? text[start - 1] : '';
    if (before && /[\w/.]/.test(before)) continue;
    if (!isChannelMention(m[1])) continue;
    if (start > cut) parts.push({ text: text.slice(cut, start) });
    parts.push({ text: m[0], login: m[1].toLowerCase() });
    cut = start + m[0].length;
  }
  if (cut < text.length) parts.push({ text: text.slice(cut) });
  return parts;
}

function accept(raw: string, into: Set<string>): void {
  const name = raw.trim();
  if (!LOGIN.test(name)) return;
  if (NOT_A_CHANNEL.has(name.toLowerCase())) return;
  into.add(name.toLowerCase());
}

/**
 * Channel logins referenced by badge text, lowercased and deduped.
 * Order is the order they appear.
 */
export function extractChannelLogins(text: string): string[] {
  const found = new Set<string>();
  if (!text) return [];

  // "/studbudz" and "twitch.tv/fps_shaka", including the "either of the
  // following channels" list. Same matcher the panel chips with, so a name that
  // gets a chip also gets a card and vice versa.
  for (const part of splitChannelMentions(text)) {
    if (part.login) accept(part.login, found);
  }

  // "Ibai's channel", "JasonTheWeen’s 7 day survival". Scraped copy uses the
  // curly apostrophe far more often than the straight one.
  for (const m of text.matchAll(/\b([a-zA-Z][a-zA-Z0-9_]{3,24})['’]s\s+(?:twitch\s+)?(?:channel|stream)\b/gi)) {
    accept(m[1], found);
  }

  // "the participating channel StudBudz", "channels Foo and Bar", "channel: Baz".
  //
  // Prose writes a handle capitalised, so requiring a capital keeps ordinary
  // sentences out ("any channel outside this category", "channels include").
  // A following capitalised word means a multi-word display name rather than a
  // login ("channels include: Riot Games"), which cannot be resolved anyway.
  //
  // Two things keep this narrow, and both were learned the hard way:
  //  - The `\b` ending each name. Without it the trailing lookahead does not
  //    reject anything, it just backtracks one character at a time until it
  //    passes, so "Bulbasaur\nCharmander" quietly yielded the login "Bulbasau"
  //    and carded a real but entirely unrelated channel.
  //  - Same line only. A colon list on the lines BELOW belongs to whatever the
  //    sentence was actually about: "...Chat Badges that require a subscription
  //    to participating channels:" followed by Bulbasaur / Charmander / Squirtle
  //    is naming badges, not channels.
  const NAME = String.raw`[A-Z][A-Za-z0-9_]{3,24}\b`;
  for (const m of text.matchAll(
    new RegExp(`\\bchannels?\\b[:\\t ]+(?:named[ \\t]+|called[ \\t]+)?(${NAME}(?:[ \\t]*(?:,|and|&)[ \\t]*${NAME})*)(?!\\s+[A-Z])`, 'g')
  )) {
    for (const part of m[1].split(/[ \t]*(?:,|and|&)[ \t]*/)) accept(part, found);
  }

  return [...found];
}
