// The stat marks and accolade icons both phones and the desktop draw are named
// by one catalog (utils/accolades.catalog.json). These read it, so a name added
// there without a mark on a platform fails here, not as a blank tile on
// someone's phone. The iPhone's map lives in Cosmetics/MemberSections.swift on
// the iPhone car; this check skips on a checkout without it.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import * as Lucide from 'lucide-react';
import { describe, expect, it } from 'vitest';
import { ACCOLADE_ICONS as DESKTOP_ACCOLADE_ICONS } from '../../components/profile/accoladeIcons';
import { ACCOLADE_ICONS, STAT_ICONS, houseSlug } from './memberIcons';
import { SECTION_TITLES } from './profileSheetParts';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const swiftPath = `${root}ios/Sources/StreamNook/Cosmetics/MemberSections.swift`;

interface Catalog {
  titles: Record<string, string>;
  stats: { key: string; label: string; icon: string }[];
  accolades: { id: string; icon: string }[];
}
const catalog = JSON.parse(readFileSync(`${root}src/utils/accolades.catalog.json`, 'utf8')) as Catalog;

const statKeys = catalog.stats.map((s) => s.key);
const accoladeIcons = [...new Set(catalog.accolades.map((a) => a.icon))];
/** "message-square" -> "MessageSquare", the name lucide-react exports. */
const pascal = (name: string) =>
  name
    .split('-')
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join('');
const lucide = Lucide as unknown as Record<string, unknown>;

describe('the catalog', () => {
  it("the phones' fixed headings are the catalog's titles (the phones read no catalog)", () => {
    expect(SECTION_TITLES.relics).toBe(catalog.titles.relics);
    expect(SECTION_TITLES.top_emotes).toBe(catalog.titles.top_emotes);
    expect(SECTION_TITLES.accolades).toBe(catalog.titles.accolades);
  });

  it('names the stats and the accolade icons', () => {
    expect(statKeys).toEqual(
      expect.arrayContaining(['hours', 'twitch_age', 'followers', 'account_type', 'messages', 'streams', 'subs_gifted', 'cosmetics', 'favorite_channel', 'member_rank', 'views']),
    );
    expect(accoladeIcons.length).toBeGreaterThan(30);
  });
});

describe('Android stat marks', () => {
  it('has a mark for exactly the stats in the catalog', () => {
    expect(Object.keys(STAT_ICONS).sort()).toEqual([...statKeys].sort());
  });

  it("draws the catalog's own icon for each stat", () => {
    for (const s of catalog.stats) {
      expect(STAT_ICONS[s.key].icon, `${s.key} should be ${pascal(s.icon)}`).toBe(lucide[pascal(s.icon)]);
    }
  });
});

describe('accolade icons', () => {
  it('Android has a mark for every icon the catalog names', () => {
    expect(accoladeIcons.filter((n) => !ACCOLADE_ICONS[n])).toEqual([]);
  });

  it('the desktop wall has a mark for every icon the catalog names', () => {
    expect(accoladeIcons.filter((n) => !DESKTOP_ACCOLADE_ICONS[n])).toEqual([]);
  });
});

describe('the iPhone marks', () => {
  it('has a mark for every stat and accolade icon the catalog names', () => {
    let swift: string;
    try {
      swift = readFileSync(swiftPath, 'utf8');
    } catch {
      return; // A checkout without the iPhone sources.
    }
    expect(statKeys.filter((k) => !swift.includes(`"${k}":`))).toEqual([]);
    expect(accoladeIcons.filter((n) => !swift.includes(`"${n}":`))).toEqual([]);
  });
});

describe('house art', () => {
  it('reads the slug of bundled house art', () => {
    expect(houseSlug('streamnook-house:streamnook-default')).toBe('streamnook-default');
    expect(houseSlug('streamnook-house:streamnook-subscriber@chat')).toBe('streamnook-subscriber');
    expect(houseSlug('https://cdn.streamnook.app/badges/x.webp')).toBeNull();
    expect(houseSlug('streamnook-house:')).toBeNull();
  });
});
