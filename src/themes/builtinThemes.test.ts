import { describe, expect, it } from 'vitest';
import { DEFAULT_THEME_ID, OLED_THEME_ID, getOledTheme, phoneFloors, themeCategories, themes } from './index';

// The built-in palettes are one JSON file the Rust core reads too
// (services/theme_palette.rs), so a malformed entry breaks every shell at once.
const PALETTE_KEYS = [
  'background', 'backgroundSecondary', 'backgroundTertiary', 'surface', 'surfaceHover', 'surfaceActive',
  'textPrimary', 'textSecondary', 'textMuted', 'accent', 'accentHover', 'accentMuted', 'accentOnSurface', 'border', 'borderLight',
  'borderSubtle', 'success', 'warning', 'error', 'info', 'scrollbarThumb', 'scrollbarTrack', 'glassOpacity',
  'glassHoverOpacity', 'glassActiveOpacity',
];
const HIGHLIGHT_KEYS = ['pink', 'purple', 'blue', 'cyan', 'green', 'yellow', 'orange', 'red'];

describe('built-in themes', () => {
  it('lists every theme once, in a known category', () => {
    expect(themes).toHaveLength(22);
    expect(new Set(themes.map((t) => t.id)).size).toBe(themes.length);
    const categories = new Set(themeCategories.map((c) => c.id));
    for (const theme of themes) expect(categories.has(theme.category)).toBe(true);
    expect(themes.some((t) => t.id === DEFAULT_THEME_ID)).toBe(true);
    expect(themes.some((t) => t.id === OLED_THEME_ID)).toBe(true);
  });

  it('gives every palette every token', () => {
    for (const theme of themes) {
      for (const key of PALETTE_KEYS) {
        expect(typeof (theme.palette as unknown as Record<string, unknown>)[key], `${theme.id}.${key}`).toBe('string');
      }
      for (const key of HIGHLIGHT_KEYS) {
        expect(typeof (theme.palette.highlight as Record<string, unknown>)[key], `${theme.id}.highlight.${key}`).toBe('string');
      }
    }
  });

  it('derives the OLED palette from its accent', () => {
    const oled = getOledTheme('#ff9933');
    expect(oled.palette.accent).toBe('#ff9933');
    expect(oled.palette.accentMuted).toBe('rgba(255, 153, 51, 0.5)');
    // A preset clears 3:1 on OLED's cards (theme_palette::every_oled_preset_clears_the_floor).
    expect(oled.palette.accentOnSurface).toBe('#ff9933');
    // The Rust core pins the same value (theme_palette::oled_follows_its_accent).
    expect(oled.palette.textSecondary).toBe('#ffcd9b');
    expect(getOledTheme('bad').palette.accent).toBe('#a064ff');
  });

  // The Rust core lifts muted text to 4.5:1 (theme_palette::lift_muted) for the shells it paints;
  // this shell paints the built-ins straight from the JSON, so they ship at the floor already and
  // every platform shows the same muted color. This twins the Rust test.
  it('ships every muted text at 4.5:1 or better against its background', () => {
    const rgba = (css: string): number[] => {
      const c = css.trim().toLowerCase();
      if (c.startsWith('#')) {
        const h = c.length === 4 ? [...c.slice(1)].map((x) => x + x).join('') : c.slice(1);
        const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
        return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
      }
      const parts = c.replace(/^rgba?\(|\)$/g, '').split(/[\s,/]+/).filter(Boolean).map(Number);
      return [parts[0], parts[1], parts[2], parts[3] ?? 1];
    };
    const lin = (v: number) => (v / 255 <= 0.04045 ? v / 255 / 12.92 : ((v / 255 + 0.055) / 1.055) ** 2.4);
    const lum = (r: number, g: number, b: number) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    for (const theme of themes) {
      const [br, bg, bb] = rgba(theme.palette.background);
      const [fr, fg, fb, a] = rgba(theme.palette.textMuted);
      const mixed = lum(fr * a + br * (1 - a), fg * a + bg * (1 - a), fb * a + bb * (1 - a));
      const back = lum(br, bg, bb);
      const contrast = (Math.max(mixed, back) + 0.05) / (Math.min(mixed, back) + 0.05);
      expect(contrast, `${theme.id} muted text`).toBeGreaterThanOrEqual(4.5);
    }
    expect(themes.find((t) => t.id === DEFAULT_THEME_ID)?.palette.textMuted).toBe('rgba(151, 177, 185, 0.75)');
  });

  // Twins theme_palette::floor_secondary: on the raised surface (surface over background), secondary
  // text reaches 4.5:1 and is never quieter than muted text.
  it('ships every secondary text at 4.5:1 on the surface, never quieter than muted', () => {
    const rgba = (css: string): number[] => {
      const c = css.trim().toLowerCase();
      if (c.startsWith('#')) {
        const h = c.length === 4 ? [...c.slice(1)].map((x) => x + x).join('') : c.slice(1);
        const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
        return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
      }
      const parts = c.replace(/^rgba?\(|\)$/g, '').split(/[\s,/]+/).filter(Boolean).map(Number);
      return [parts[0], parts[1], parts[2], parts[3] ?? 1];
    };
    const over = (fg: number[], bg: number[]) => [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3]));
    const lin = (v: number) => (v / 255 <= 0.04045 ? v / 255 / 12.92 : ((v / 255 + 0.055) / 1.055) ** 2.4);
    const lum = (c: number[]) => 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
    const ratio = (a: number[], b: number[]) =>
      (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
    for (const theme of themes) {
      const track = over(rgba(theme.palette.surface), rgba(theme.palette.background));
      const secondary = ratio(over(rgba(theme.palette.textSecondary), track), track);
      const muted = ratio(over(rgba(theme.palette.textMuted), track), track);
      expect(secondary, `${theme.id} secondary text`).toBeGreaterThanOrEqual(4.5);
      expect(secondary, `${theme.id} secondary vs muted`).toBeGreaterThanOrEqual(muted);
    }
    // OLED's accent-derived secondary: black is the darkest an accent can mix to.
    const oled = getOledTheme('#000000').palette;
    const oledTrack = over(rgba(oled.surface), rgba(oled.background));
    expect(ratio(over(rgba(oled.textSecondary), oledTrack), oledTrack)).toBeGreaterThanOrEqual(4.5);
  });
});

// A phone shows nothing behind the app, so a see-through custom background is
// flattened over black and the tertiary floor over that page; surfaces built
// as solid from them then are solid.
describe('phone floors', () => {
  it('flattens a see-through background over black and tertiary over the page', () => {
    const floors = phoneFloors('rgba(40, 60, 80, 0.5)', 'rgba(200, 200, 200, 0.5)');
    expect(floors.background).toBe('#141e28');
    expect(floors.tertiary).toBe('#6e7378');
  });

  it('leaves opaque floors as they are', () => {
    expect(phoneFloors('#0c0c0d', '#1a1a1b')).toEqual({ background: '#0c0c0d', tertiary: '#1a1a1b' });
  });
});
