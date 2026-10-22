// Run with: npm test
//
// Chat brought up over the player's full screen has to paint over the video,
// and so does everything chat opens. Three ways it has broken, each pinned:
// the box holding a full-screen player became a stacking context (the column
// drew under the video), the column carried a backdrop-filter (user cards
// opened from chat were laid out inside the column), and chat's page-level
// menus kept their window z-index (they opened under the video).

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const src = resolve(__dirname, '..');
const css = readFileSync(resolve(src, 'styles/globals.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** Every declaration block whose selector list is exactly `selector`. */
const blocks = (selector: string): string[] => {
  const out: string[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  for (let m = re.exec(css); m; m = re.exec(css)) {
    if (m[1].trim() === selector) out.push(m[2]);
  }
  return out;
};

describe('chat over the player full screen', () => {
  it('the box holding a full-screen player is not a stacking context', () => {
    const rules = blocks('.video-player-container:has(.plyr--fullscreen-fallback)');
    expect(rules.length, 'the full-screen player box rule moved; point this test at it').toBeGreaterThan(0);
    for (const body of rules) {
      const z = body.match(/z-index:\s*([^;]+)/)?.[1].trim();
      if (z !== undefined) expect(z).toBe('auto !important');
    }
  });

  it('the chat column itself never becomes a containing block for its children', () => {
    const rules = blocks('.chat-fullscreen-overlay');
    expect(rules.length, 'the chat column rule moved; point this test at it').toBeGreaterThan(0);
    for (const body of rules) {
      expect(body).not.toMatch(/(^|[;\s])(-webkit-)?backdrop-filter\s*:/);
      expect(body).not.toMatch(/(^|[;\s])filter\s*:/);
      expect(body).not.toMatch(/(^|[;\s])transform\s*:/);
      expect(body).not.toMatch(/(^|[;\s])opacity\s*:/);
    }
  });

  it('the chat column sits over the video and under the player panels', () => {
    const scale = blocks(':root').join('\n');
    const value = (name: string) => Number(scale.match(new RegExp(`${name}:\\s*(\\d+)`))?.[1]);
    expect(value('--sn-z-player-fs-chat')).toBeGreaterThan(value('--sn-z-player-fs'));
    expect(value('--sn-z-player-fs-popover')).toBeGreaterThan(value('--sn-z-player-fs-chat'));
    const lift = Number(blocks('html[data-player-fullscreen]').join('').match(/--sn-z-over-player-lift:\s*(\d+)/)?.[1]);
    expect(lift).toBeGreaterThan(value('--sn-z-player-fs-popover'));
  });

  it.each([
    'components/chat/ChatHeaderMenu.tsx',
    'components/ui/Dropdown.tsx',
    'components/EmoteCard.tsx',
    'components/chat/CommandMenu.tsx',
    'components/chat/BlendSourcesButton.tsx',
  ])('%s lifts its page-level layer over the full-screen player', (file) => {
    expect(readFileSync(resolve(src, file), 'utf8')).toContain('--sn-z-over-player-lift');
  });

  it('the chat button shows only in the player full screen with a chat to bring up', () => {
    expect(blocks('.video-player-container .plyr__control[data-streamnook-fschat]').join('')).toMatch(/display:\s*none/);
    expect(
      blocks(
        'html[data-fs-chat-available] .video-player-container .plyr--fullscreen-fallback .plyr__control[data-streamnook-fschat]',
      ).join(''),
    ).toMatch(/display:\s*inline-flex/);
  });
});
