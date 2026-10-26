// Run with: npm test
//
// No live player's <video> element starts on its own: playback starts when the
// page's buffer gate says so, never from an `autoPlay` attribute.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const components = resolve(__dirname, '..');
const read = (file: string) =>
  readFileSync(resolve(components, file), 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '');

describe('no live player video autoplays', () => {
  it.each(['multi-nook/MultiNookCell.tsx', 'VideoPlayer.tsx'])('%s has a video element and none has autoPlay', (file) => {
    const src = read(file);
    const starts = [...src.matchAll(/<video\b/g)].map((m) => m.index ?? 0);
    expect(starts.length).toBeGreaterThan(0);
    for (const start of starts) {
      // The element's attributes run to the first self-closing `/>` after it.
      const end = src.indexOf('/>', start);
      expect(end).toBeGreaterThan(start);
      expect(src.slice(start, end)).not.toMatch(/\bautoPlay\b/i);
    }
  });
});
