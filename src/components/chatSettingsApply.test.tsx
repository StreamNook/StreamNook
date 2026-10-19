// @vitest-environment node
// The phone's Chat settings reach its chat: a change is saved to Rust as a
// patch and kept in the store, and the real chat list and rows (the ones the
// Android app draws) render what the store holds. Rows read the store through
// selectors, so a change shows on an open chat without reopening it.
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

const invoke = vi.hoisted(() => vi.fn((..._args: unknown[]) => Promise.resolve(null)));
vi.mock('@tauri-apps/api/core', () => ({ invoke, convertFileSrc: (p: string) => p, Channel: class {} }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})), emit: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: class {}, getCurrentWebviewWindow: () => ({ label: 'main' }) }));
// The Android app's render path.
vi.mock('../utils/platform', async (original) => ({ ...(await original<typeof import('../utils/platform')>()), IS_MOBILE: true }));
// Moderator line steps come from an external store with no server snapshot;
// they play no part in how rows look here.
vi.mock('../services/moderation', async (original) => ({
  ...(await original<typeof import('../services/moderation')>()),
  useModSliderSteps: () => null,
}));

import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import ChatMessage from './ChatMessage';
import ChatMessageList from './ChatMessageList';
import { useAppStore } from '../stores/AppStore';
import type { ChatDesignSettings, Settings } from '../types';

function row(id: string, tags: Record<string, string> = {}, metadata: Record<string, unknown> = {}) {
  return {
    id,
    user_id: '42',
    username: 'nookviewer',
    display_name: 'NookViewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '2000',
    content: `row ${id}`,
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '42', id, ...tags },
    segments: [{ type: 'text', content: `row ${id}` }],
    metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, ...metadata },
  };
}

const RAID = row('raid', { 'msg-id': 'raid', 'system-msg': '12 raiders from Someone have joined!' }, { msg_type: 'raid' });

// Server rendering reads the store's INITIAL state, so the settings go there.
function withSettings(design: Partial<ChatDesignSettings>, events: Record<string, unknown> = {}) {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { ...(initial.settings.chat_design as object), ...design };
  initial.settings.chat_events = events;
}

function renderRow(design: Partial<ChatDesignSettings>): string {
  withSettings(design);
  return renderToStaticMarkup(createElement(ChatMessage, { message: row('one') as never }));
}

function renderList(design: Partial<ChatDesignSettings>, events: Record<string, unknown> = {}): string {
  withSettings(design, events);
  const noop = () => {};
  const props = {
    messages: [row('a'), RAID, row('b')],
    renderToken: 1,
    isPaused: false,
    onScroll: noop,
    onUsernameClick: noop,
    onReplyClick: noop,
    onEmoteRightClick: noop,
    onUsernameRightClick: noop,
    onBadgeClick: noop,
    highlightedMessageId: null,
    deletedMessageIds: new Set(),
    clearedUserContexts: new Map(),
    emotes: null,
    getMessageId: (m: unknown) => (typeof m === 'string' ? null : (m as { id: string }).id),
  };
  return renderToStaticMarkup(createElement(ChatMessageList, props as never));
}

describe('a Chat settings change is saved', () => {
  it('goes to Rust as a patch of the chat design and stays in the store', async () => {
    const before = useAppStore.getState().settings;
    const next = {
      ...before,
      chat_design: { ...before.chat_design, show_dividers: false, message_entrance: 'rise' },
    } as Settings;
    await useAppStore.getState().updateSettings(next);

    const call = invoke.mock.calls.find(([cmd]) => cmd === 'patch_settings');
    expect(call).toBeDefined();
    const patch = (call![1] as { patch: Record<string, ChatDesignSettings> }).patch;
    expect(patch.chat_design?.show_dividers).toBe(false);
    expect(patch.chat_design?.message_entrance).toBe('rise');
    expect(useAppStore.getState().settings.chat_design?.message_entrance).toBe('rise');
    expect(useAppStore.getState().settings.chat_design?.show_dividers).toBe(false);
  });
});

describe('the chat draws what the Chat page set', () => {
  it('Dividers: a line over each row when on, none when off', () => {
    expect(renderRow({ show_dividers: true })).toContain('border-t border-borderSubtle');
    expect(renderRow({ show_dividers: false })).not.toContain('border-t border-borderSubtle');
  });

  it("Text weight: moves the body from the font's own chat weight, Normal leaves it", () => {
    expect(renderRow({ font_weight: 400 })).toContain('calc(var(--chat-body-weight, 300) + 0)');
    expect(renderRow({ font_weight: 700 })).toContain('calc(var(--chat-body-weight, 300) + 300)');
    expect(renderRow({ font_weight: 300 })).toContain('calc(var(--chat-body-weight, 300) - 100)');
  });

  it('Message animation: the list carries the chosen arrival, and none when None', () => {
    for (const entrance of ['fade', 'slide', 'rise'] as const) {
      expect(renderList({ message_entrance: entrance })).toContain(`data-entrance="${entrance}"`);
    }
    expect(renderList({ message_entrance: 'none' })).not.toContain('data-entrance');
  });

  it('Alternating backgrounds: the list stripes its rows', () => {
    expect(renderList({ alternating_backgrounds: true })).toContain('chat-striped');
    expect(renderList({ alternating_backgrounds: false })).not.toContain('chat-striped');
  });

  it('Chat events: a hidden kind is left out on its platform', () => {
    const raidRow = 'data-message-id="raid"';
    expect(renderList({}, {})).toContain(raidRow);
    expect(renderList({}, { hidden_provider_events: ['twitch:raid'] })).not.toContain(raidRow);
    expect(renderList({}, { hidden_provider_events: ['kick:raid'] })).toContain(raidRow);
    // The ordinary rows around it stay either way.
    expect(renderList({}, { hidden_provider_events: ['twitch:raid'] })).toContain('data-message-id="b"');
  });
});
