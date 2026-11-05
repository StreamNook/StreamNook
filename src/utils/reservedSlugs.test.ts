// reservedSlugs.json is the app's copy of the words streamnook.app keeps for
// itself (src/lib/reservedSlugs.ts in that repo): a first path segment that is
// one of them is never a member's page. The copy must equal the site's list.
// The sibling checkout is found as SN_SITE_DIR, then C:/streamnook.app, then
// /mnt/c/streamnook.app; a machine with none fails, except in CI.
import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import file from './reservedSlugs.json';

const LIST = 'src/lib/reservedSlugs.ts';
const VALID_LOGIN = /^[a-z0-9_]{1,25}$/;

/** The words in the site's RESERVED_SLUGS, in its order (the site's own build check reads it the same way). */
const siteWords = (source: string): string[] => {
  const body = /new Set\(\[([\s\S]*?)\]\)/.exec(source)?.[1] ?? '';
  return [...body.replace(/\/\/.*$/gm, '').matchAll(/'([^']+)'/g)].map((m) => m[1]);
};

describe('reservedSlugs.json', () => {
  it('holds lowercase login-shaped words, each once', () => {
    expect(file.slugs.length).toBeGreaterThan(50);
    expect(new Set(file.slugs).size).toBe(file.slugs.length);
    for (const word of file.slugs) expect(VALID_LOGIN.test(word), word).toBe(true);
  });

  it("equals the site's list, in the site's order", () => {
    const dir = [process.env.SN_SITE_DIR, 'C:/streamnook.app', '/mnt/c/streamnook.app'].find((d) => d && existsSync(`${d}/${LIST}`));
    if (!dir) {
      if (process.env.CI) return;
      throw new Error(`no streamnook.app checkout with ${LIST} found: set SN_SITE_DIR, or CI to skip`);
    }
    expect(file.slugs).toStrictEqual(siteWords(readFileSync(`${dir}/${LIST}`, 'utf8')));
  });
});
