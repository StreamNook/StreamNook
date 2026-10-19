// The kept Chats screen, behind another tab, must not paint or take a touch,
// with rooms open and with none (the picker). It once painted over Following
// and Browse: the `hidden` attribute lost to the `flex` utility beside it.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it } from 'vitest';
import { KeptSurface, keptSurfaceClass } from './KeptSurface';
import { ChatsScreen } from './screens/ChatsScreen';
import { useChatTabsStore } from './chat/chatTabsStore';

// Tailwind display utilities: any one of them beside the `hidden` attribute
// outweighs it (same specificity, utilities come later).
const DISPLAY = new Set(['flex', 'inline-flex', 'grid', 'inline-grid', 'block', 'inline-block', 'inline', 'contents', 'table', 'flow-root']);

const initial = useChatTabsStore.getState();
afterEach(() => useChatTabsStore.setState(initial, true));

function rooms(...channels: string[]) {
  useChatTabsStore.setState({
    tabs: channels.map((c) => ({
      channel: c,
      provider: 'twitch',
      login: c,
      label: c,
      avatar: null,
      channelId: '1',
      pinnedToStream: false,
      live: true,
      mentions: 0,
    })) as never,
    activeChannel: channels[0] ?? null,
  });
}

/** The surface's own tag: its attributes and class tokens. */
function surface(shown: boolean) {
  const html = renderToStaticMarkup(
    <KeptSurface shown={shown}>
      <ChatsScreen shown={shown} />
    </KeptSurface>,
  );
  const open = html.slice(0, html.indexOf('>') + 1);
  const classes = (/class="([^"]*)"/.exec(open)?.[1] ?? '').split(/\s+/).filter(Boolean);
  return { html, open, classes };
}

describe('the kept Chats screen', () => {
  for (const [state, setup, marker] of [
    ['with rooms open', () => rooms('kathi', 'summit1g'), 'kathi'],
    ['with no rooms (the picker)', () => rooms(), 'Search any channel'],
  ] as const) {
    it(`paints nothing and takes no touch behind another tab, ${state}`, () => {
      setup();
      const { html, open, classes } = surface(false);
      // The real screen is in there, in the state under test.
      expect(html).toContain(marker);
      expect(classes).toContain('hidden');
      expect(classes.filter((c) => DISPLAY.has(c))).toEqual([]);
      expect(open).toMatch(/\shidden=""/);
      expect(open).toMatch(/\sinert=""/);
      expect(open).toMatch(/\saria-hidden="true"/);
    });

    it(`shows and takes touches in front, ${state}`, () => {
      setup();
      const { open, classes } = surface(true);
      expect(classes).toContain('flex');
      expect(classes).not.toContain('hidden');
      expect(open).not.toMatch(/\s(hidden|inert|aria-hidden)=/);
    });
  }

  it('is how the shell keeps Chats', () => {
    const shell = readFileSync(resolve(__dirname, 'MobileApp.tsx'), 'utf8');
    expect(shell).toMatch(/<KeptSurface\b[^>]*\bshown=\{chatsShown\}[^>]*>\s*<ChatsScreen shown=\{chatsShown\} \/>/);
  });

  it('never pairs a display utility with the hidden state', () => {
    expect(keptSurfaceClass(false).split(' ').filter((c) => DISPLAY.has(c))).toEqual([]);
    expect(keptSurfaceClass(true).split(' ')).toContain('flex');
  });
});
