// The channel-key and watch-URL rules exist in both languages. This file and
// the Rust tests in providers/key.rs and providers/watch_urls.rs read the same
// fixtures, so the twins cannot drift apart silently.
import { describe, expect, it } from 'vitest';
import fixtures from './providerTwins.fixtures.json';
import { makeKey, normalizeChannel, parseKey } from './providerKey';
import { buildProviderUrl } from './streamProvider';
import { favoriteIdOf } from './favorites';
import type { ProviderId } from '../types/providers';

describe('provider twins', () => {
  it('normalize channels the way Rust does', () => {
    for (const [p, c, want] of fixtures.normalize) expect(normalizeChannel(p as ProviderId, c)).toBe(want);
  });

  it('make and parse keys the way Rust does', () => {
    for (const [p, c, want] of fixtures.make_key) expect(makeKey(p as ProviderId, c)).toBe(want);
    for (const [key, provider, channel] of fixtures.parse_key) expect(parseKey(key)).toEqual({ provider, channel });
  });

  it('build watch URLs the way Rust does', () => {
    for (const [p, c, want] of fixtures.watch_url) expect(buildProviderUrl(p as ProviderId, c)).toBe(want);
  });

  it('name favorites the way Rust does', () => {
    const cases = fixtures.favorite_id as unknown as [string, string, string, string | null][];
    expect(cases.length).toBeGreaterThan(0);
    for (const [p, id, login, want] of cases) {
      expect(favoriteIdOf({ provider: p as ProviderId, user_id: id, user_login: login }), `${p} ${id} ${login}`).toBe(want);
    }
  });
});
