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

// SN-274: Twitch wraps every notice a partner channel shares in msg-id
// `sharedchatnotice` and names the real kind in `source-msg-id`. A shared
// announcement drew as "subscribed!" because the wrapper alone made the sub card.
function shared(sourceMsgId: string, extraTags: Record<string, string> = {}) {
  return {
    id: `n-${sourceMsgId}`,
    user_id: '42',
    username: 'leeo_h',
    display_name: 'leeo_h',
    color: '#22c55e',
    badges: [],
    timestamp: '2000',
    content: 'Spenden fuer Herzenswuensche',
    provider: 'twitch',
    channel: 'partner',
    tags: {
      id: `n-${sourceMsgId}`,
      'user-id': '42',
      'display-name': 'leeo_h',
      login: 'leeo_h',
      'msg-id': 'sharedchatnotice',
      'source-msg-id': sourceMsgId,
      'room-id': '100',
      'source-room-id': '200',
      ...extraTags,
    },
    segments: [{ type: 'text', content: 'Spenden fuer Herzenswuensche' }],
    metadata: {
      is_action: false,
      is_mentioned: false,
      is_first_message: false,
      is_from_shared_chat: true,
      source_room_id: '200',
      msg_type: sourceMsgId,
      shown_name: 'leeo_h',
    },
  };
}

function render(message: unknown): string {
  const html = renderToStaticMarkup(createElement(ChatMessage, { message: message as never }));
  return html.replace(/<svg.*?<\/svg>/g, '').replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');
}

describe('a notice shared from a partner channel (sharedchatnotice)', () => {
  it('an announcement reads as an announcement, never as a sub', () => {
    const text = render(shared('announcement', { 'msg-param-color': 'PRIMARY' }));
    expect(text).toContain('Announcement');
    expect(text).toContain('Spenden fuer Herzenswuensche');
    expect(text).not.toMatch(/subscribed/i);
  });

  it('a shared resub still draws the sub card', () => {
    const text = render(
      shared('resub', {
        'msg-param-cumulative-months': '6',
        'msg-param-sub-plan': '1000',
        'system-msg': 'leeo_h subscribed at Tier 1. They have subscribed for 6 months!',
      }),
    );
    expect(text).toMatch(/subscribed/i);
    expect(text).not.toContain('Announcement');
  });
});
