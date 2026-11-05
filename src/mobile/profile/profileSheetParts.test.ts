import { describe, expect, it } from 'vitest';
import type { AtmosphereView, MemberRank, PageLink, TopEmoteView } from '../../types/profileView';
import {
  EMOTE_TILE_LIMIT,
  SECTION_TITLES,
  accentBorder,
  accentTint,
  accountGlyph,
  auraFor,
  emoteTiles,
  frameInsets,
  hexRgb,
  newOpenId,
  panelFor,
  plateSlot,
  statPanel,
  statValueSize,
  tzOffsetMin,
} from './profileSheetParts';

const link: PageLink = {
  href: 'https://streamnook.app/u/tyler',
  url: 'https://streamnook.app/tyler',
  prefix: 'streamnook.app/',
  name: 'tyler',
  accent: '#22d3ee',
};

describe('plateSlot', () => {
  it('shows a link whatever else is true', () => {
    for (const registryLoaded of [true, false])
      for (const isMember of [true, false])
        for (const fresh of [true, false]) expect(plateSlot(registryLoaded, isMember, link, fresh)).toBe('shown');
  });

  it('hides once the fresh answer landed with no link', () => {
    expect(plateSlot(true, true, null, true)).toBe('hidden');
    expect(plateSlot(false, true, null, true)).toBe('hidden');
    expect(plateSlot(false, false, null, true)).toBe('hidden');
  });

  it('holds the slot before the fresh answer: for a member, or while the list has not loaded', () => {
    expect(plateSlot(true, true, null, false)).toBe('reserved');
    expect(plateSlot(false, false, null, false)).toBe('reserved');
    expect(plateSlot(false, true, null, false)).toBe('reserved');
  });

  it('holds nothing for a loaded list that says this is not a member', () => {
    expect(plateSlot(true, false, null, false)).toBe('hidden');
  });
});

describe('auraFor', () => {
  const atmosphere = { id: 'aurora', name: 'Aurora', accent: '1, 2, 3', swatch: '', baseColor: '#000' } as AtmosphereView;
  const rank: MemberRank = { number: 7, tier: 'mythic', label: 'Mythic', accent: '#FBBF24', aura: 0.13 };

  it('prefers the worn Atmosphere, carrying the accent', () => {
    expect(auraFor('atmosphere', atmosphere, '#8B5CF6', rank)).toEqual({ kind: 'atmosphere', color: '#8B5CF6' });
    expect(auraFor('atmosphere', atmosphere, undefined, rank)).toEqual({ kind: 'atmosphere' });
  });

  it('falls back to the rank aura with its color and strength', () => {
    expect(auraFor('tier', null, '#FBBF24', rank)).toEqual({ kind: 'aura', color: '#FBBF24', alpha: 0.13 });
    expect(auraFor('paint', undefined, undefined, rank)).toEqual({ kind: 'aura', color: '#FBBF24', alpha: 0.13 });
  });

  it('is nothing without either', () => {
    expect(auraFor(undefined, null, undefined, null)).toEqual({ kind: 'none' });
  });
});

describe('frameInsets', () => {
  it('maps the border to the four sides: vertical top and bottom, horizontal left and right', () => {
    expect(frameInsets({ border: [18, 14] }, 1)).toEqual({ top: 18, right: 14, bottom: 18, left: 14 });
  });

  it('scales them', () => {
    expect(frameInsets({ border: [18, 14] }, 2)).toEqual({ top: 36, right: 28, bottom: 36, left: 28 });
  });
});

describe('panelFor', () => {
  it('an accolade: the label without soft hyphens, and its hint', () => {
    expect(panelFor('accolade', { label: 'Com­ple­tion­ist', hint: 'Earn every accolade' })).toEqual({
      title: 'Completionist',
      detail: 'Earn every accolade',
    });
  });

  it('an emote: its name, and the count with the provider in capitals', () => {
    expect(panelFor('emote', { name: 'Kappa', count_text: '1,234', provider: 'twitch' })).toEqual({
      title: 'Kappa',
      detail: '1,234 · TWITCH',
    });
  });

  it('a badge: the title (else the name), and its description', () => {
    expect(panelFor('badge', { title: 'Subscriber', description: 'Subscribed for 3 months' })).toEqual({
      title: 'Subscriber',
      detail: 'Subscribed for 3 months',
    });
    expect(panelFor('badge', { name: 'vip' })).toEqual({ title: 'vip', detail: '' });
    expect(panelFor('badge', {})).toEqual({ title: 'Badge', detail: '' });
  });

  it('a paint and an Atmosphere', () => {
    expect(panelFor('paint', { name: 'Gold Leaf' })).toEqual({ title: 'Gold Leaf', detail: '7TV paint' });
    expect(panelFor('atmosphere', { name: 'Aurora' })).toEqual({ title: 'Aurora', detail: 'Atmosphere' });
  });
});

describe('emoteTiles', () => {
  const emote = (i: number): TopEmoteView => ({
    id: `e${i}`,
    name: `E${i}`,
    provider: 'twitch',
    url: `https://cdn/e${i}.png`,
    count: 100 - i,
    count_text: String(100 - i),
  });
  // Rust sends up to 16 emotes with art.
  const many = Array.from({ length: 16 }, (_, i) => emote(i));

  it('takes twelve of the sixteen', () => {
    expect(emoteTiles(many, new Set())).toHaveLength(EMOTE_TILE_LIMIT);
    expect(EMOTE_TILE_LIMIT).toBe(12);
    expect(emoteTiles(many, new Set())[11].id).toBe('e11');
  });

  it('drops failed art before the cap, so the next emote fills in', () => {
    const tiles = emoteTiles(many, new Set(['e0', 'e3']));
    expect(tiles).toHaveLength(12);
    expect(tiles.map((e) => e.id)).not.toContain('e0');
    expect(tiles[0].id).toBe('e1');
    expect(tiles[11].id).toBe('e13');
  });

  it('shows what is left when more than four fail', () => {
    const tiles = emoteTiles(many, new Set(['e0', 'e1', 'e2', 'e3', 'e4']));
    expect(tiles).toHaveLength(11);
    expect(tiles[10].id).toBe('e15');
  });

  it('shows fewer when there are fewer', () => {
    expect(emoteTiles(many.slice(0, 3), new Set(['e1'])).map((e) => e.id)).toEqual(['e0', 'e2']);
  });
});

describe('accent helpers', () => {
  it('reads #rrggbb', () => {
    expect(hexRgb('#8B5CF6')).toEqual([139, 92, 246]);
    expect(hexRgb('8b5cf6')).toEqual([139, 92, 246]);
    expect(hexRgb('#fff')).toBeNull();
    expect(hexRgb(undefined)).toBeNull();
  });

  it('tints and borders only with a valid accent', () => {
    expect(accentTint('#8B5CF6', 0.3)).toBe('rgba(139, 92, 246, 0.3)');
    expect(accentBorder('#8B5CF6', 0.14)).toEqual({ borderColor: 'rgba(139, 92, 246, 0.14)' });
    expect(accentTint('nope', 0.3)).toBeUndefined();
    expect(accentBorder(undefined, 0.3)).toBeUndefined();
  });
});

describe('stat tile panel', () => {
  it('says the detail under the label, and nothing for a tile without one', () => {
    expect(statPanel({ label: 'Twitch age', detail: 'Joined March 20, 2014' })).toEqual({
      title: 'Twitch age',
      detail: 'Joined March 20, 2014',
    });
    expect(statPanel({ label: 'Followers' })).toBeNull();
  });
});

describe('section titles', () => {
  it('are the three headings Rust does not send', () => {
    expect(SECTION_TITLES).toEqual({ relics: 'Relics', top_emotes: 'Top Emotes', accolades: 'Accolades' });
  });
});

describe('tile text', () => {
  it('steps the value size down as it grows, never below 13', () => {
    expect(statValueSize('1,234')).toBe(17);
    expect(statValueSize('12,345,678')).toBe(15);
    expect(statValueSize('1,234,567,890')).toBe(13);
    expect(statValueSize('x'.repeat(30))).toBe(13);
  });

  it('picks the account mark from the value', () => {
    expect(accountGlyph('Partner')).toBe('verified');
    expect(accountGlyph('Twitch Staff')).toBe('verified');
    expect(accountGlyph('Affiliate')).toBe('twitch');
    expect(accountGlyph('Viewer')).toBe('award');
  });
});

describe('tzOffsetMin', () => {
  it('is east positive: getTimezoneOffset is west positive', () => {
    expect(tzOffsetMin({ getTimezoneOffset: () => 480 })).toBe(-480); // UTC-8
    expect(tzOffsetMin({ getTimezoneOffset: () => -330 })).toBe(330); // UTC+5:30
    expect(tzOffsetMin({ getTimezoneOffset: () => 0 })).toBe(0);
  });
});

describe('newOpenId', () => {
  it('gives a different id each time', () => {
    const ids = new Set(Array.from({ length: 20 }, () => newOpenId()));
    expect(ids.size).toBe(20);
    for (const id of ids) expect(id.length).toBeGreaterThan(8);
  });
});
