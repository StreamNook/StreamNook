// The member page on streamnook.app runs its own copy of accolades.cases.json
// against its own wall rules. The two copies must hold the same cases, or a
// rule could change here and pass there. The sibling checkout is found the same
// way everywhere: SN_SITE_DIR, then C:/streamnook.app, then /mnt/c/streamnook.app.
// A machine with none of them fails, except in CI, which builds one repo alone.
import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const COPY = 'functions/utils/profile/accolades.cases.json';

const siteDir = (): string | null => {
  const tries = [process.env.SN_SITE_DIR, 'C:/streamnook.app', '/mnt/c/streamnook.app'];
  for (const dir of tries) if (dir && existsSync(`${dir}/${COPY}`)) return dir;
  return null;
};

describe('the site keeps the same accolade cases', () => {
  it('holds a copy equal to this one, value for value', () => {
    const dir = siteDir();
    if (!dir) {
      if (process.env.CI) return;
      throw new Error(`no streamnook.app checkout with ${COPY} found: set SN_SITE_DIR, or CI to skip`);
    }
    const here = JSON.parse(readFileSync(new URL('./accolades.cases.json', import.meta.url), 'utf8'));
    const there = JSON.parse(readFileSync(`${dir}/${COPY}`, 'utf8'));
    expect(there).toStrictEqual(here);
  });
});
