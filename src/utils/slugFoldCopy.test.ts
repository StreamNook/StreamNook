// The slug fold table lives in two repos: src-tauri/tests/fixtures/slug_fold.json here
// (the Rust slug test reads it) and tests/fixtures/slug_fold.json in Penrose's repo (its
// link test reads it there, so that suite reads nothing outside its own repo). A
// regeneration (streamnook.app's scripts/gen-slug-fold.ts) must update both, byte for byte.
// The bot checkout is found as SN_BOT_DIR, then C:/StreamNookBot, then
// /mnt/c/StreamNookBot; a machine with none fails, except in CI.
import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const APP_COPY = 'src-tauri/tests/fixtures/slug_fold.json';
const BOT_COPY = 'tests/fixtures/slug_fold.json';

describe('slug_fold.json', () => {
  it("is byte-equal to Penrose's copy", () => {
    const dir = [process.env.SN_BOT_DIR, 'C:/StreamNookBot', '/mnt/c/StreamNookBot'].find((d) => d && existsSync(`${d}/${BOT_COPY}`));
    if (!dir) {
      if (process.env.CI) return;
      throw new Error(`no StreamNookBot checkout with ${BOT_COPY} found: set SN_BOT_DIR, or CI to skip`);
    }
    const same = readFileSync(`${dir}/${BOT_COPY}`).equals(readFileSync(APP_COPY));
    expect(same, `${dir}/${BOT_COPY} differs from ${APP_COPY}: copy the app's file over Penrose's`).toBe(true);
  });
});
