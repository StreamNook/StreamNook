// @vitest-environment node
import { beforeAll, describe, expect, it, vi } from 'vitest';

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

const MOD_ART = 'https://static-cdn.jtvnw.net/badges/v1/mod/1';
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string) =>
    Promise.resolve(
      cmd === 'get_cached_global_badges'
        ? { data: [{ set_id: 'moderator', versions: [{ id: '1', title: 'Moderator', image_url_1x: MOD_ART, image_url_2x: MOD_ART, image_url_4x: MOD_ART }] }] }
        : null,
    ),
  ),
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
import { initializeBadgeCache } from '../services/twitchBadges';

// A chatter's right-to-left letters or bidi controls (U+202E RIGHT-TO-LEFT
// OVERRIDE above all) must stay inside their own name or text: the row keeps
// badges, then name, then body, left to right.
const RLO = '‮';

function row(name: string, body: string) {
  return {
    id: 'bidi',
    user_id: '1',
    username: 'viewer',
    display_name: name,
    color: '#FF69B4',
    badges: [{ name: 'moderator', version: '1' }],
    timestamp: '1000',
    content: body,
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'bidi', badges: 'moderator/1', 'display-name': name },
    segments: [{ type: 'text', content: body }],
    metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, shown_name: name },
  };
}

function render(name: string, body: string): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, font_size: 14 };
  return renderToStaticMarkup(createElement(ChatMessage, { message: row(name, body) as never }));
}

/** Where the badge strip, the name's island and the body's island start. */
function order(html: string, name: string, body: string) {
  return {
    badges: html.indexOf('sn-badge-strip'),
    name: html.indexOf(`<bdi>${name}</bdi>`),
    body: html.indexOf(`<bdi><span>${body}</span></bdi>`),
  };
}

beforeAll(async () => {
  await initializeBadgeCache();
});

describe('a chat row with right-to-left text', () => {
  const cases: Array<[string, string, string]> = [
    ['a normal row', 'kingjabas', 'and the 2027 nba champs are playing tonight'],
    ['an override in the name', `${RLO}8 )detirehnI(`, 'why you in bed'],
    ['an override in the body', 'plain_pat', `${RLO}deb ni uoy yhw`],
    ['an Arabic name', 'محمد', 'hello chat'],
    ['an Arabic body', 'kingjabas', 'مرحبا بالجميع 123 كيف حالكم'],
  ];

  it.each(cases)('%s keeps badges, name and body in order, each piece isolated', (_label, name, body) => {
    const html = render(name, body);
    const at = order(html, name, body);
    expect(at.badges).toBeGreaterThanOrEqual(0);
    // The name and the body are each their own <bdi>, so nothing inside them
    // can reorder the row around them.
    expect(at.name).toBeGreaterThan(at.badges);
    expect(at.body).toBeGreaterThan(at.name);
  });

  it('keeps right-to-left text whole inside its island', () => {
    const body = 'مرحبا بالجميع';
    const html = render('kingjabas', body);
    expect(html).toContain(`<bdi><span>${body}</span></bdi>`);
  });
});
