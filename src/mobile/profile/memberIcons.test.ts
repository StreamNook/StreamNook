// Rust names a member's stats and accolade icons (services/member_profile.rs);
// both phones map those names to marks. These read the Rust and Swift sources
// so a name added on one side without the others fails here, not as a blank
// medallion on someone's phone.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { ACCOLADE_ICONS, STAT_ICONS, houseSlug } from './memberIcons';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const rust = readFileSync(`${root}src-tauri/src/services/member_profile.rs`, 'utf8');
const swiftPath = `${root}ios/Sources/StreamNook/Cosmetics/MemberSections.swift`;

/** Every lucide name the accolade list uses: `def(id, label, icon, ...)`. */
const rustIcons = [...rust.matchAll(/def\("[^"]+", "[^"]+", "([a-z0-9-]+)"/g)].map((m) => m[1]);
/** Every stat key `build` can serve: `tile("key", ...)`. */
const rustStats = [...rust.matchAll(/tile\("([a-z_]+)"/g)].map((m) => m[1]);

describe('member sheet marks', () => {
  it('reads the names out of the Rust source', () => {
    expect(rustIcons.length).toBeGreaterThan(30);
    expect(rustStats).toEqual(
      expect.arrayContaining(['hours', 'messages', 'streams', 'favorite_channel', 'twitch_age', 'followers', 'views']),
    );
  });

  it('has an Android mark for every name Rust serves', () => {
    expect(rustIcons.filter((n) => !ACCOLADE_ICONS[n])).toEqual([]);
    expect(rustStats.filter((k) => !STAT_ICONS[k])).toEqual([]);
  });

  it('has an iPhone mark for every name Rust serves', () => {
    let swift: string;
    try {
      swift = readFileSync(swiftPath, 'utf8');
    } catch {
      return; // A checkout without the iPhone sources.
    }
    expect(rustIcons.filter((n) => !swift.includes(`"${n}":`))).toEqual([]);
    expect(rustStats.filter((k) => !swift.includes(`"${k}":`))).toEqual([]);
  });

  it('reads the slug of bundled house art', () => {
    expect(houseSlug('streamnook-house:streamnook-default')).toBe('streamnook-default');
    expect(houseSlug('streamnook-house:streamnook-subscriber@chat')).toBe('streamnook-subscriber');
    expect(houseSlug('https://cdn.streamnook.app/badges/x.webp')).toBeNull();
    expect(houseSlug('streamnook-house:')).toBeNull();
  });
});
