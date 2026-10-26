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

// The emote hover preview opens through the Tooltip, which clones the emote's
// element to add its hover and focus props. 8.8.16 wrapped every chat emote in
// a component that drew its <img> without them, so no emote ever showed its
// preview. A text tooltip also names its child (aria-label), which a static
// render can see: the name reaching the <img> proves the Tooltip's props do.
function render(segments: unknown[]): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, compact_emote_tooltips: true };
  const message = {
    id: 'tt',
    user_id: '1',
    username: 'viewer',
    display_name: 'Viewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '1000',
    content: 'unused',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'tt' },
    segments,
    metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, shown_name: 'Viewer' },
  };
  return renderToStaticMarkup(createElement(ChatMessage, { message: message as never }));
}

const imgTag = (html: string, alt: string) => html.match(/<img\b[^>]*>/g)?.find((t) => t.includes(`alt="${alt}"`));

describe('chat emotes carry their hover preview', () => {
  it('a 7TV emote receives the Tooltip props', () => {
    const tag = imgTag(render([{ type: 'emote', content: 'Pog7tv', emote_id: '60ae958e229664e8667aea38', emote_url: 'https://cdn.7tv.app/emote/60ae958e229664e8667aea38/2x.avif' }]), 'Pog7tv');
    expect(tag).toBeDefined();
    expect(tag).toMatch(/aria-(label|description)="Pog7tv"/);
  });

  it('a BetterTTV emote receives the Tooltip props', () => {
    const tag = imgTag(render([{ type: 'emote', content: 'catJAM', emote_id: '5f1b0186cf6d2144653d2970', emote_url: 'https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/1x' }]), 'catJAM');
    expect(tag).toBeDefined();
    expect(tag).toMatch(/aria-(label|description)="catJAM"/);
  });
});
