import { describe, expect, it } from 'vitest';
import { DEFAULT_THEME_ID, OLED_THEME_ID, getOledTheme, themeCategories, themes } from './index';

// The built-in palettes are one JSON file the Rust core reads too
// (services/theme_palette.rs), so a malformed entry breaks every shell at once.
const PALETTE_KEYS = [
  'background', 'backgroundSecondary', 'backgroundTertiary', 'surface', 'surfaceHover', 'surfaceActive',
  'textPrimary', 'textSecondary', 'textMuted', 'accent', 'accentHover', 'accentMuted', 'border', 'borderLight',
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
    expect(getOledTheme('bad').palette.accent).toBe('#a064ff');
  });
});
