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

  it('builds only the public wall: no own flag, no clock, no creation date', () => {
    expect(call?.[1]).not.toMatch(/\bown\b|\btoday\b|\bcreated_at\b/);
  });

  it('draws your own wall from Rust, and the signed-in wall is read only for your own profile', () => {
    expect(source).toMatch(/useOwnAccolades\(isOwnProfile \? userId : null\)/);
    expect(source).toMatch(/isOwnProfile\s*\?\s*ownWall\.wall/);
  });

  it('no longer saves accolades itself: Rust grants', () => {
    expect(source).not.toMatch(/\bgrantAccolade\b/);
    expect(source).not.toMatch(/\bGRANTABLE_IDS\b|\bgrantKey\b/);
  });

  it('keeps no accolade list of its own', () => {
    expect(source).not.toMatch(/\bbaseAccolades\b|\bsecretAccolades\b|\bSEASONAL_PRES\b/);
  });
});
