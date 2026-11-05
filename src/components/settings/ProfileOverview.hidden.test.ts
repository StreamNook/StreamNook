// A public profile must never print a number its member hid. The wall decides
// that from the member's hidden keys, so the Overview has to hand them over.
// A node test cannot render the component, so this pins the call in its source.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(new URL('./ProfileOverview.tsx', import.meta.url), 'utf8').replace(/\r\n/g, '\n');

describe('ProfileOverview hands the wall what it needs', () => {
  const call = /buildAccoladeWall\(\{([^}]*)\}\)/.exec(source);

  it('builds its wall through buildAccoladeWall', () => {
    expect(call).not.toBeNull();
  });

  it("passes the member's hidden keys", () => {
    expect(call?.[1]).toMatch(/\bhidden: hiddenSections\b/);
  });

  it('says whose wall it is', () => {
    expect(call?.[1]).toMatch(/\bown: isOwnProfile\b/);
  });

  it('keeps no accolade list of its own', () => {
    expect(source).not.toMatch(/\bbaseAccolades\b|\bsecretAccolades\b|\bSEASONAL_PRES\b/);
  });
});
