import { describe, expect, it } from 'vitest';
import { matchesCosmetic } from './cosmeticSearch';

describe('matchesCosmetic', () => {
  it('matches everything while the search is empty', () => {
    expect(matchesCosmetic('Uranium', '')).toBe(true);
    expect(matchesCosmetic(undefined, '')).toBe(true);
  });

  it('ignores case, accents and extra spacing', () => {
    expect(matchesCosmetic('Pokémon Legends', 'pokemon')).toBe(true);
    expect(matchesCosmetic('Valentine’s Gifting 2026', 'GIFTING   2026')).toBe(true);
    expect(matchesCosmetic('Rosé', 'rose')).toBe(true);
  });

  it('finds a part of the name and nothing else', () => {
    expect(matchesCosmetic('Fire & Ice', 'ice')).toBe(true);
    expect(matchesCosmetic('Uranium', 'plutonium')).toBe(false);
    expect(matchesCosmetic(null, 'x')).toBe(false);
  });
});
