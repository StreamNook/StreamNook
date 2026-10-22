// The kept Chats screen, behind another tab, must not paint or take a touch,
// with rooms open and with none (the picker). It once painted over Following
// and Browse: the `hidden` attribute lost to the `flex` utility beside it.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';

// The real Chats screen's imports read a few browser globals as they load.
vi.hoisted(() => {
  const noop = () => {};
  const g = globalThis as Record<string, unknown>;
  const store = new Map<string, string>();
  g.localStorage ??= {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  };
  g.WebSocket ??= class {};
  g.window ??= {
    location: { hash: '', search: '', href: 'http://localhost/', pathname: '/' },
    addEventListener: noop,
    removeEventListener: noop,
    dispatchEvent: () => true,
    matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop }),
    localStorage: g.localStorage,
    innerWidth: 390,
    innerHeight: 844,
  };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null), convertFileSrc: (p: string) => p }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
// Sheets portal to the page body, which a server render has none of; they are
// closed in every state here.
vi.mock('react-dom', async (original) => ({ ...(await original<typeof import('react-dom')>()), createPortal: () => null }));
// The chat list reads stores a server render cannot; the screen's own header,
// room strip and pins stay real.
vi.mock('./chat/MobileChatPane', () => ({ MobileChatPane: () => <div data-chat-pane="">rows and composer</div> }));
import { KeptSurface, keptSurfaceClass } from './KeptSurface';
import { ChatsScreen } from './screens/ChatsScreen';
import { useChatTabsStore } from './chat/chatTabsStore';

// Tailwind display utilities: any one of them beside the `hidden` attribute
// outweighs it (same specificity, utilities come later).
const DISPLAY = new Set(['flex', 'inline-flex', 'grid', 'inline-grid', 'block', 'inline-block', 'inline', 'contents', 'table', 'flow-root']);

const initial = { ...useChatTabsStore.getState() };
afterEach(() => {
  Object.assign(useChatTabsStore.getInitialState(), { tabs: initial.tabs, activeChannel: initial.activeChannel });
  useChatTabsStore.setState(initial, true);
});

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
  // A server render reads a store's initial state, not its current one.
  const { tabs, activeChannel } = useChatTabsStore.getState();
  Object.assign(useChatTabsStore.getInitialState(), { tabs, activeChannel });
}

/** The surface's own tag: its attributes and class tokens. */
function surface(shown: boolean) {
  // Only while rendering: the sheets name the page body as their portal target
  // (the portal itself renders nothing here, see above).
  const g = globalThis as Record<string, unknown>;
  const hadDocument = 'document' in g;
  if (!hadDocument) g.document = { body: {} };
  let html: string;
  try {
    html = renderToStaticMarkup(
      <KeptSurface shown={shown}>
        <ChatsScreen shown={shown} />
      </KeptSurface>,
    );
  } finally {
    if (!hadDocument) delete g.document;
  }
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
    expect(shell).toMatch(/<KeptSurface\b[^>]*\bshown=\{chatsShown\}[^>]*>\s*<ChatsScreen shown=\{chatsShown\}[^>]*\/>/);
  });

  it('never pairs a display utility with the hidden state', () => {
    expect(keptSurfaceClass(false).split(' ').filter((c) => DISPLAY.has(c))).toEqual([]);
    expect(keptSurfaceClass(true).split(' ')).toContain('flex');
  });
});
