import { describe, expect, it } from 'vitest';
import { ANY_LANGUAGE, DISCOVERY_LANGUAGE_OPTIONS, nextDiscoveryLanguages } from './discoveryLanguages';

describe('nextDiscoveryLanguages', () => {
  it('picking Any language clears the languages', () => {
    expect(nextDiscoveryLanguages(['en', 'es'], ['en', 'es', ANY_LANGUAGE])).toEqual([ANY_LANGUAGE]);
  });

  it('picking a language while Any is on drops Any', () => {
    expect(nextDiscoveryLanguages([ANY_LANGUAGE], [ANY_LANGUAGE, 'fr'])).toEqual(['fr']);
  });

  it('plain picks pass through, empty included', () => {
    expect(nextDiscoveryLanguages(['en'], ['en', 'pt'])).toEqual(['en', 'pt']);
    expect(nextDiscoveryLanguages(['en'], [])).toEqual([]);
    expect(nextDiscoveryLanguages([ANY_LANGUAGE], [])).toEqual([]);
  });

  it('offers Any language first and every language once', () => {
    expect(DISCOVERY_LANGUAGE_OPTIONS[0].value).toBe(ANY_LANGUAGE);
    const values = DISCOVERY_LANGUAGE_OPTIONS.map((o) => o.value);
    expect(new Set(values).size).toBe(values.length);
  });
});
