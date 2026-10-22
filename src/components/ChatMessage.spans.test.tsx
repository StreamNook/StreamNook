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

// Mentions and links arrive from Rust as their own segments
// (services/text_spans.rs); the row draws them as they come and runs no
// pattern of its own over the text.
function row(segments: unknown[]) {
  return {
    id: 'spans',
    user_id: '1',
    username: 'viewer',
    display_name: 'Viewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '1000',
    content: 'unused',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'spans' },
    segments,
    metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, shown_name: 'Viewer' },
  };
}

function render(segments: unknown[], design: Record<string, unknown> = {}): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, ...design };
  return renderToStaticMarkup(createElement(ChatMessage, { message: row(segments) as never }));
}

describe('mention and link segments', () => {
  it('draws a spaced name as one mention', () => {
    const html = render([
      { type: 'text', content: 'gg ' },
      { type: 'mention', content: '@Call of Duty', login: 'callofduty' },
      { type: 'text', content: '!' },
    ]);
    expect(html).toContain('>@Call of Duty<');
    expect(html).not.toContain('>@Call</span>');
  });

  it('leaves an @ in plain text alone (Rust decides what is a mention)', () => {
    const html = render([{ type: 'text', content: 'mail me at a@b' }]);
    // One plain span: no mention cut out of it on the page.
    expect(html).toContain('<span>mail me at a@b</span>');
  });

  it('draws a link with the short label Rust stamped, and the full href', () => {
    const link = { type: 'link', content: 'https://www.example.com/a', url: 'https://www.example.com/a', label: 'example.com/a' };
    const short = render([link]);
    expect(short).toContain('href="https://www.example.com/a"');
    expect(short).toContain('>example.com/a<');
    const full = render([link], { shorten_links: false });
    expect(full).toContain('>https://www.example.com/a<');
  });

  it('a bare-domain link Rust found is a link too', () => {
    const html = render([{ type: 'link', content: 'test.fr', url: 'https://test.fr', label: 'test.fr' }]);
    expect(html).toContain('href="https://test.fr"');
  });
});
