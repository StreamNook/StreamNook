// Run with: npm test
//
// Plyr's poster is a full-size, transparent layer over the <video>. If it takes
// pointer events, a click on the picture never reaches the video, and the
// click-to-pause route on the video element never runs.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const css = readFileSync(resolve(__dirname, 'globals.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** Every declaration block whose selector list is exactly `selector`. */
const blocks = (selector: string): string[] => {
  const out: string[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  for (let m = re.exec(css); m; m = re.exec(css)) {
    if (m[1].trim() === selector) out.push(m[2]);
  }
  return out;
};

describe('the player poster', () => {
  it('takes no pointer events, so clicks reach the video under it', () => {
    const rules = blocks('.video-player-container .plyr__poster');
    expect(rules.length, 'the poster rule moved; point this test at it').toBeGreaterThan(0);
    expect(rules.some((body) => /pointer-events:\s*none/.test(body))).toBe(true);
  });
});
