// The search over Profile > Customize's badge and paint walls. It filters the
// walls already on screen as you type: a Rust round trip per key would only
// add latency to a list the page already holds.

/** Lowercased, accents and spacing folded, so "pokemon" finds "Pokémon". */
const fold = (s: string) =>
  s
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/\s+/g, ' ')
    .trim();

/** Whether a cosmetic's name matches the search (an empty search matches all). */
export function matchesCosmetic(name: string | null | undefined, query: string): boolean {
  if (!query) return true;
  return fold(name ?? '').includes(fold(query));
}
