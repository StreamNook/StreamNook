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

const TIME = '12:41 PM';

function row() {
  return {
    id: 'time',
    user_id: '1',
    username: 'viewer',
    display_name: 'Viewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '1000',
    content: 'hello there',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'time' },
    segments: [{ type: 'text', content: 'hello there' }],
    metadata: {
      is_action: false,
      is_mentioned: false,
      is_first_message: false,
      is_from_shared_chat: false,
      shown_name: 'Viewer',
      formatted_timestamp: TIME,
      formatted_timestamp_with_seconds: '12:41:09 PM',
    },
  };
}

function render(design: Record<string, unknown>): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, font_size: 14, ...design };
  return renderToStaticMarkup(createElement(ChatMessage, { message: row() as never }));
}

// The element that carries the chat font size is the row's text line.
const ROW_OPEN = /<div[^>]*style="[^"]*font-size:14px;line-height:1\.625[^"]*"[^>]*>/;

describe('the chat timestamp', () => {
  it('sits in the same row element as the name, ahead of it', () => {
    const html = render({ show_timestamps: true });
    const open = html.match(ROW_OPEN);
    expect(open).not.toBeNull();
    const line = html.slice((open?.index ?? 0) + (open?.[0].length ?? 0));
    const time = line.indexOf(TIME);
    const name = line.indexOf('Viewer');
    expect(time).toBeGreaterThanOrEqual(0);
    expect(name).toBeGreaterThan(time);
    // Nothing closes the row between the time and the name: one inline flow.
    expect(line.slice(0, name)).not.toContain('</div>');
  });

  it('is not drawn when timestamps are off', () => {
    const html = render({ show_timestamps: false });
    expect(html).not.toContain(TIME);
    expect(html).toContain('Viewer');
  });
});
