// An on/off option set is a list of switch lines, never a cluster of toggle
// chips. These pin the chip markup out of the settings panels that had one.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const FILES = ['ChatSettings.tsx', 'HighlightAppearanceSettings.tsx'];

describe('no on/off chip clusters', () => {
  for (const file of FILES) {
    it(`${file} uses switch lists`, () => {
      const src = readFileSync(resolve(__dirname, file), 'utf8');
      expect(src).not.toContain('aria-pressed={on}');
      expect(src).not.toContain('glass-button-static text-textMuted');
      expect(src).not.toContain("glass-button text-textSecondary hover:text-textPrimary'}");
      expect(src).toContain('<SwitchList');
    });
  }

  // The overlay builder keeps chip styling for its single-choice size presets,
  // so its four on/off sets are pinned by the clicks their chips made.
  it('OverlayBuilder.tsx uses switch lists for its on/off sets', () => {
    const src = readFileSync(resolve(__dirname, '../overlay/builder/OverlayBuilder.tsx'), 'utf8');
    for (const chipClick of [
      'onClick={() => toggleTwitchBadgeCategory(',
      'onClick={() => toggleBadgeProvider(',
      'onClick={() => toggleProviderEvent(',
      'onClick={() => hasSource && toggleSourceFilter(',
    ]) {
      expect(src).not.toContain(chipClick);
    }
    expect(src.split('<SwitchList').length - 1).toBeGreaterThanOrEqual(4);
  });
});
