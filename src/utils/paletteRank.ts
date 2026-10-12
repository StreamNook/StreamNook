// How the Ctrl+K palette ranks a search. Pure, so the rule is tested without
// the palette's sources (stores, Tauri).

import type { PaletteItem, PaletteSection } from './commandPaletteSources';

/** An exact match: the whole title, or a channel's login or display name. */
export const EXACT_MATCH_SCORE = 1000;
/** The query is exactly a snippet's alias. */
export const EXACT_ALIAS_SCORE = 1500;

export function scoreItem(item: PaletteItem, queryLower: string): number {
  if (!queryLower) return 0;

  // Alias matches win, hard. A user explicitly bound this snippet to this
  // keystroke; we trust that intent over any title-text fuzz.
  if (item.alias) {
    if (item.alias === queryLower) return EXACT_ALIAS_SCORE;
    if (item.alias.startsWith(queryLower)) return 1200 - (item.alias.length - queryLower.length);
  }

  const title = item.title.toLowerCase();
  const subtitle = (item.subtitle ?? '').toLowerCase();
  const keywords = item.keywords ?? '';

  if (title === queryLower) return EXACT_MATCH_SCORE;
  // A channel typed by its login (or a display name the title does not carry).
  if (item.exactNames?.some((n) => !!n && n.toLowerCase() === queryLower)) return EXACT_MATCH_SCORE;
  if (title.startsWith(queryLower)) return 700 - (title.length - queryLower.length);
  const titleIdx = title.indexOf(queryLower);
  if (titleIdx !== -1) return 500 - titleIdx;
  if (subtitle.includes(queryLower)) return 300;
  if (keywords.includes(queryLower)) return 200;

  const haystack = `${title} ${subtitle} ${keywords}`;
  let lastIdx = -1;
  let gaps = 0;
  for (const ch of queryLower) {
    const nextIdx = haystack.indexOf(ch, lastIdx + 1);
    if (nextIdx === -1) return -1;
    if (lastIdx !== -1) gaps += nextIdx - lastIdx - 1;
    lastIdx = nextIdx;
  }
  return 100 - Math.min(gaps, 80);
}

/**
 * The order the sections of a search render in. A section holding an exact
 * match (a title, a channel's login or name, a snippet's alias) goes first, so
 * Enter on the top row opens the channel you typed rather than a Quick Action
 * whose words happen to contain its letters. Among those, `prefer` decides
 * (a channel over a recent chatter of the same name); everything else keeps
 * `base` order. With no query nothing is scored and `base` is unchanged.
 */
export function orderSections(
  base: readonly PaletteSection[],
  items: readonly Pick<PaletteItem, 'section' | 'score'>[],
  prefer: readonly PaletteSection[] = [],
): PaletteSection[] {
  const exact = new Set<PaletteSection>();
  for (const it of items) {
    if (it.score === EXACT_MATCH_SCORE || it.score === EXACT_ALIAS_SCORE) exact.add(it.section);
  }
  if (exact.size === 0) return [...base];
  const rank = (s: PaletteSection) => {
    const i = prefer.indexOf(s);
    return i === -1 ? prefer.length : i;
  };
  const first = base.filter((s) => exact.has(s)).sort((a, b) => rank(a) - rank(b));
  return [...first, ...base.filter((s) => !exact.has(s))];
}
