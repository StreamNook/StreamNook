// The plate's wiring, pinned in source: the hero asks about the member's own
// Twitch login, the card has no second link button, the old URL builder is gone,
// and the plate's floor is the floor Rust lifts the accent against.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  PLATE_CHECK,
  PLATE_EDGE,
  PLATE_FLOOR,
  PLATE_FLOOR_ALPHA,
  PLATE_FLOOR_RGB,
  PLATE_MUTED,
  PLATE_RESERVED_WIDTH,
} from './plateStyle';
import { profileLinkAsk } from './profileLink';

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

describe('a Kick member card links to the member own Twitch page', () => {
  it('asks with the member id and the login Twitch gave for it', () => {
    // A claimed Kick chatter: the card was opened as "kickfan", the member is
    // Twitch id 676 whose login is "twitchfan".
    expect(profileLinkAsk('676', 'twitchfan')).toEqual({ memberId: '676', login: 'twitchfan' });
  });

  it('the hero passes the member id and view.info.login, never a card login', () => {
    const hero = read('src/components/profile/MemberProfileHero.tsx');
    expect(hero).toContain('useProfileLink({ memberId: userId, login: info?.login');
    expect(hero).not.toMatch(/\busername\b/);
  });

  it('the card gives the hero the member id (cardMemberId), not the chat platform id', () => {
    const card = read('src/components/UserProfileCard.tsx');
    expect(card).toContain('<MemberProfileHero userId={cardMemberId}');
  });
});

describe('one link surface per card', () => {
  const needle = ['build', 'ProfileUrl'].join('');

  it('the old URL builder and the Profile link button are gone', () => {
    const offenders = walk(resolve(root, 'src'))
      .filter((f) => !/[\\/]profileLinkWiring\.test\.ts$/.test(f))
      .filter((f) => readFileSync(f, 'utf8').includes(needle));
    expect(offenders).toEqual([]);
    const card = read('src/components/UserProfileCard.tsx');
    expect(card).not.toContain('Profile link');
    expect(card).not.toContain('profileLinkCopied');
  });

  it('the hook asks again when the member list changes', () => {
    const hook = read('src/components/profile/useProfileLink.ts');
    expect(hook).toContain('[memberId, login, registryVersion]');
  });

  it('Settings > Profile draws the plate under the login for your own account', () => {
    const settings = read('src/components/settings/ProfileSettings.tsx');
    expect(settings).toContain('useProfileLink({');
    expect(settings).toMatch(/@\{currentUser\.login\}<\/p>\s*<ProfileNameplate/);
  });
});

describe('the plate floor', () => {
  it('is one constant: the string, the channels and the alpha agree', () => {
    const [r, g, b] = PLATE_FLOOR_RGB;
    expect(PLATE_FLOOR).toBe(`rgba(${r},${g},${b},${PLATE_FLOOR_ALPHA})`);
  });

  it('matches the floor Rust lifts the accent against (member_link PLATE_FLOOR)', () => {
    const rust = read('src-tauri/src/services/member_link.rs');
    const m = /PLATE_FLOOR:\s*\[u8;\s*4\]\s*=\s*\[(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\]/.exec(rust);
    expect(m, 'PLATE_FLOOR not found in member_link.rs').not.toBeNull();
    const [, r, g, b, a] = m!.map(Number);
    expect([r, g, b]).toEqual([...PLATE_FLOOR_RGB]);
    expect(Number((a / 255).toFixed(2))).toBe(PLATE_FLOOR_ALPHA);
  });

  it('the plate draws that constant and no theme token', () => {
    const plate = read('src/components/profile/ProfileNameplate.tsx');
    expect(plate).toContain('background: PLATE_FLOOR');
    expect(plate).not.toMatch(/var\(--color/);
    expect(plate).not.toMatch(/glass-(panel|modal|button)/);
  });

  it('keeps the planner constants: reserved width 214, check color #4ade80', () => {
    expect(PLATE_RESERVED_WIDTH).toBe(214);
    expect(PLATE_CHECK).toBe('#4ade80');
  });
});

// The iPhone plate (ios/Sources/StreamNook/Design/ProfileNameplate.swift on the
// iPhone car) keeps the same literals in its doc comments. There is no `ios/` on
// `dev`, so this runs on the car and skips here.
describe.skipIf(!existsSync(resolve(root, 'ios')))('the iPhone plate literals', () => {
  it('names the floor, the edge and the muted color', () => {
    const swift = read('ios/Sources/StreamNook/Design/ProfileNameplate.swift');
    expect(swift).toContain(PLATE_FLOOR);
    expect(swift).toContain(PLATE_EDGE);
    expect(swift).toContain(PLATE_MUTED);
  });
});
