// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';

vi.hoisted(() => {
  const store = new Map<string, string>();
  const storage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, String(v)),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
    key: () => null,
    length: 0,
  };
  const noop = () => {};
  const el = () => ({ style: {}, dataset: {} as Record<string, string>, setAttribute: noop, appendChild: noop, classList: { add: noop, remove: noop, toggle: noop }, addEventListener: noop, removeEventListener: noop });
  const g = globalThis as Record<string, unknown>;
  g.localStorage = storage;
  g.sessionStorage = storage;
  g.document = { documentElement: el(), body: el(), head: el(), createElement: el, addEventListener: noop, removeEventListener: noop, querySelector: () => null, querySelectorAll: () => [], visibilityState: 'visible', fonts: { ready: Promise.resolve() } };
  g.window = Object.assign(globalThis, {
    location: { hash: '', href: 'http://localhost/', origin: 'http://localhost', search: '' },
    addEventListener: noop,
    removeEventListener: noop,
    matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop, addListener: noop, removeListener: noop }),
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 0),
    cancelAnimationFrame: noop,
    __TAURI_INTERNALS__: { invoke: () => Promise.resolve(null), transformCallback: () => 0, metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } } },
  });
  if (!g.navigator) g.navigator = { userAgent: 'node', platform: 'Win32', language: 'en-US' };
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
  convertFileSrc: (p: string) => p,
  Channel: class {},
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})), emit: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: class {}, getCurrentWebviewWindow: () => ({ label: 'main' }) }));

import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import ChatMessage from './ChatMessage';
import { useAppStore } from '../stores/AppStore';
import { MOD_BTTV_WIDE, MOD_HIDDEN, MOD_PREFIX } from '../utils/emoteModifiers';

// A stacked group (a base with zero-width overlays or modifiers on it) draws
// at the same size and spacing as the base would on its own. It used to be a
// fixed rem size, so any chat text size above 14px shrank the base.
function row(segments: unknown[]) {
  return {
    id: 'zw',
    user_id: '1',
    username: 'viewer',
    display_name: 'Viewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '1000',
    content: 'unused',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'zw' },
    segments,
    metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, shown_name: 'Viewer' },
  };
}

function render(segments: unknown[]): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false };
  return renderToStaticMarkup(createElement(ChatMessage, { message: row(segments) as never }));
}

const INLINE = 'calc(2em * var(--sn-emote-scale, 1))';
const base = { type: 'emote', content: 'BaseEmote', emote_id: '60ae958e229664e8667aea38', emote_url: 'https://cdn.7tv.app/emote/60ae958e229664e8667aea38/2x.avif' };
const overlay = { type: 'emote', content: 'RainTime', emote_id: '60ae9b3b229664e866803d39', emote_url: 'https://cdn.7tv.app/emote/60ae9b3b229664e866803d39/2x.avif', is_zero_width: true };
const wide = { type: 'emote', content: 'w!', emote_id: 'wide', emote_url: 'https://cdn.betterttv.net/emote/wide/1x', modifier_flags: MOD_PREFIX | MOD_BTTV_WIDE | MOD_HIDDEN };
const space = { type: 'text', content: ' ' };

// Attribute order is React's to choose, so find the tag first, then its height.
const imgTag = (html: string, alt: string) => html.match(/<img\b[^>]*>/g)?.find((t) => t.includes(`alt="${alt}"`));
const heightOf = (html: string, alt: string) => imgTag(html, alt)?.match(/height:([^;"]+)/)?.[1];

describe('stacked emotes keep the base size', () => {
  it('a base under a zero-width emote is as tall as the same emote alone', () => {
    expect(heightOf(render([base]), 'BaseEmote')).toBe(INLINE);
    expect(heightOf(render([base, space, overlay]), 'BaseEmote')).toBe(INLINE);
  });

  it('the zero-width emote draws at its base size', () => {
    const html = render([base, space, overlay]);
    expect(heightOf(html, 'RainTime')).toBe(heightOf(html, 'BaseEmote'));
  });

  it('a BetterTTV w! target keeps the inline height', () => {
    expect(heightOf(render([wide, space, base]), 'BaseEmote')).toBe(INLINE);
  });

  it('a stack keeps the emote spacing setting', () => {
    const grid = render([base, space, overlay]).match(/<span[^>]*inline-grid[^>]*>/)?.[0];
    expect(grid).toContain('var(--sn-emote-margin, 0.125rem)');
  });
});
