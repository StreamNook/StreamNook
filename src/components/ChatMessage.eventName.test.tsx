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

// A localized chatter (もちパンダ / mochi_panda) with a saved nickname, rendered
// by the real ChatMessage. The watch-streak notice carries the login in
// `display-name` and opens its system-msg with the login, as Twitch sends it.
const ID = '700700';
const LOCALIZED = 'もちパンダ';
const SHOWN = `${LOCALIZED} (mochi_panda)`;
const STREAK_TEXT = 'mochi_panda watched 3 consecutive streams and sparked a watch streak!';

function row(id: string, tags: Record<string, string>, extra: Record<string, unknown> = {}) {
  return {
    id,
    user_id: ID,
    username: 'mochi_panda',
    display_name: LOCALIZED,
    color: '#FF69B4',
    badges: [],
    timestamp: '2000',
    content: 'yay',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': ID, id, ...tags },
    segments: [{ type: 'text', content: 'yay' }],
    metadata: {
      is_action: false,
      is_mentioned: false,
      is_first_message: false,
      is_from_shared_chat: false,
      shown_name: SHOWN,
      ...extra,
    },
  };
}

const ROWS = {
  plain: row('plain', { 'display-name': LOCALIZED }),
  'watch streak': row(
    'streak',
    {
      'display-name': 'mochi_panda',
      login: 'mochi_panda',
      'msg-id': 'viewermilestone',
      'msg-param-category': 'watch-streak',
      'msg-param-value': '3',
      'msg-param-copoReward': '350',
      'system-msg': STREAK_TEXT,
    },
    { msg_type: 'viewermilestone', system_message: STREAK_TEXT },
  ),
  'sub card': row(
    'sub',
    {
      'display-name': LOCALIZED,
      login: 'mochi_panda',
      'msg-id': 'resub',
      'msg-param-cumulative-months': '6',
      'msg-param-sub-plan': '1000',
      'system-msg': `${LOCALIZED} subscribed at Tier 1. They've subscribed for 6 months!`,
    },
    { msg_type: 'resub' },
  ),
  raid: row(
    'raid',
    {
      'display-name': LOCALIZED,
      login: 'mochi_panda',
      'msg-id': 'raid',
      'msg-param-viewerCount': '12',
      'system-msg': `12 raiders from ${LOCALIZED} have joined!`,
    },
    { msg_type: 'raid' },
  ),
  'reply header': {
    ...row('reply', { 'display-name': 'Other', 'user-id': '1' }),
    user_id: '1',
    username: 'other',
    display_name: 'Other',
    metadata: {
      is_action: false,
      is_mentioned: false,
      is_first_message: false,
      is_from_shared_chat: false,
      shown_name: 'Other',
      reply_info: {
        parent_msg_id: 'p',
        parent_display_name: LOCALIZED,
        parent_msg_body: 'hi',
        parent_user_id: ID,
        parent_user_login: 'mochi_panda',
        parent_shown_name: SHOWN,
      },
    },
  },
};

const STYLES = ['cards', 'outline', 'plain'] as const;
const TEMPLATES: Record<string, Record<string, string> | undefined> = {
  'platform wording': undefined,
  '{default}': { milestone: '{default}', subscription: '{default}', raid: '{default}' },
  '{username} wording': {
    milestone: '{username} watched {streak} streams',
    subscription: '{username} subbed for {months} months',
    raid: '{username} raided with {viewers}',
  },
};

function render(message: unknown, nickname: string | null, events: Record<string, unknown>): string {
  // Server rendering reads the store's INITIAL state (useSyncExternalStore's
  // server snapshot), so the settings go there.
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_customization = nickname
    ? { user_overrides: { [ID]: { user_id: ID, username: 'mochi_panda', nickname } } }
    : {};
  initial.settings.chat_events = events;
  const html = renderToStaticMarkup(createElement(ChatMessage, { message: message as never }));
  return html.replace(/<svg.*?<\/svg>/g, '').replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');
}

describe('a localized chatter with a nickname, on every row type', () => {
  for (const style of STYLES) {
    for (const [label, templates] of Object.entries(TEMPLATES)) {
      for (const [kind, message] of Object.entries(ROWS)) {
        it(`${kind}, ${style} events, ${label}: shows the nickname, never the login`, () => {
          const text = render(message, 'Cake', { event_style: style, event_templates: templates });
          expect(text).toContain('Cake');
          // The raw login never stands in for the chatter's name.
          expect(text).not.toMatch(/mochi_panda watched|mochi_panda raided|mochi_panda subbed/);
          if (kind === 'watch streak') {
            // Named once, in the header; the wording under it does not repeat it.
            expect(text.match(/Cake/g)).toHaveLength(1);
            expect(text).toContain('+350');
          }
        });
      }
    }
  }

  it('without a nickname, the watch streak and the plain row show one shown name', () => {
    const streak = render(ROWS['watch streak'], null, {});
    const plain = render(ROWS.plain, null, {});
    expect(streak).toContain(SHOWN);
    expect(plain).toContain(SHOWN);
    expect(streak).not.toMatch(/mochi_panda watched/);
  });

  it('{default} on a watch streak reads as the platform sentence without the login', () => {
    const text = render(ROWS['watch streak'], 'Cake', { event_templates: { milestone: '{default}' } });
    expect(text).toContain('Cake +350 watched 3 consecutive streams and sparked a watch streak!');
  });
});
