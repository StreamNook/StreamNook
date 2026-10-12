// The overlay renderer's Twitch badge categories must match the backend's table
// (TWITCH_BADGE_CATEGORY_SETS in src-tauri/src/models/chat_layout.rs). The
// renderer keeps its own copy because the hosted overlay page runs without the
// backend; this reads the Rust table and fails on any drift.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { TWITCH_BADGE_CATEGORIES, clampOverlayStyle, DEFAULT_OVERLAY_STYLE, twitchBadgeCategory } from './overlayConfig';

const root = resolve(__dirname, '..', '..', '..');

function rustTable(): Map<string, string[]> {
  const src = readFileSync(resolve(root, 'src-tauri/src/models/chat_layout.rs'), 'utf8');
  const start = src.indexOf('pub const TWITCH_BADGE_CATEGORY_SETS');
  if (start < 0) throw new Error('TWITCH_BADGE_CATEGORY_SETS not found in chat_layout.rs');
  const block = src.slice(start, src.indexOf('];', start));
  const out = new Map<string, string[]>();
  for (const m of block.matchAll(/\("([^"]+)",\s*&\[([^\]]*)\]\)/g)) {
    out.set(m[1], [...m[2].matchAll(/"([^"]+)"/g)].map((s) => s[1]));
  }
  return out;
}

describe('Twitch badge categories', () => {
  it('match the backend table exactly', () => {
    const rust = rustTable();
    expect(rust.size).toBeGreaterThan(0);
    const ts = new Map(TWITCH_BADGE_CATEGORIES.filter((c) => c.id !== 'other').map((c) => [c.id, c.sets]));
    expect(ts).toEqual(rust);
    expect(TWITCH_BADGE_CATEGORIES.some((c) => c.id === 'other')).toBe(true);
  });

  it('classify set ids', () => {
    expect(twitchBadgeCategory('predictions')).toBe('predictions');
    expect(twitchBadgeCategory('founder')).toBe('subscriber');
    expect(twitchBadgeCategory('sub-gift-leader')).toBe('gifted-subs');
    expect(twitchBadgeCategory('bits-leader')).toBe('bits');
    expect(twitchBadgeCategory('blossom-badge')).toBe('blossom');
    expect(twitchBadgeCategory('bloom-badge')).toBe('bloom');
    expect(twitchBadgeCategory('lead_moderator')).toBe('roles');
    expect(twitchBadgeCategory('premium')).toBe('other');
    expect(twitchBadgeCategory('broadcaster')).toBeNull();
    expect(twitchBadgeCategory('')).toBeNull();
  });

  it('keep only known ids when a config is loaded', () => {
    const style = clampOverlayStyle({
      ...DEFAULT_OVERLAY_STYLE,
      hiddenTwitchBadgeCategories: ['predictions', 'nope', 'predictions', 'roles'],
    });
    expect(style.hiddenTwitchBadgeCategories).toEqual(['predictions', 'roles']);
    const legacy = { ...DEFAULT_OVERLAY_STYLE } as Partial<typeof DEFAULT_OVERLAY_STYLE>;
    delete legacy.hiddenTwitchBadgeCategories;
    expect(clampOverlayStyle(legacy as typeof DEFAULT_OVERLAY_STYLE).hiddenTwitchBadgeCategories).toEqual([]);
  });
});
