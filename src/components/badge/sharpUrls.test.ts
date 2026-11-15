// Run with: npm test
//
// The app's twin of the site's sharp URL rule, checked against the one fixture
// every twin shares (src-tauri/tests/fixtures holds the app's byte copy).

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { SHARP_ORIGIN, sharpUrls } from './sharpUrls';

interface FixtureCase {
  a: string;
  large?: string;
  small?: string;
  out?: null;
}

const fixture = JSON.parse(
  readFileSync(resolve(__dirname, '../../../src-tauri/tests/fixtures/badgeSharpArt.fixture.json'), 'utf8'),
) as { origin: string; cases: FixtureCase[] };

describe('sharpUrls', () => {
  it('uses the fixture origin', () => {
    expect(SHARP_ORIGIN).toBe(fixture.origin);
  });

  it('matches every fixture case', () => {
    expect(fixture.cases.length).toBeGreaterThan(2);
    for (const c of fixture.cases) {
      if ('out' in c) expect(sharpUrls(c.a), JSON.stringify(c.a)).toBeNull();
      else expect(sharpUrls(c.a), c.a).toEqual({ large: c.large, small: c.small });
    }
  });

  it('gives nothing for a value that is not a string', () => {
    expect(sharpUrls(undefined)).toBeNull();
    expect(sharpUrls(null)).toBeNull();
    expect(sharpUrls(42)).toBeNull();
  });
});
