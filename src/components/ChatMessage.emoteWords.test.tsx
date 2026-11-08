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

const emote = (name: string) => ({
  type: 'emote',
  content: name,
  emote_id: '01GB4CK01800090V9B3D8CGEEX',
  emote_url: 'https://cdn.7tv.app/emote/01GB4CK01800090V9B3D8CGEEX/2x.avif',
});
const SENTENCE = [{ type: 'text', content: 'that was ' }, emote('EZ'), { type: 'text', content: ' for him' }];
const EMOTES_ONLY = [emote('EZ'), { type: 'text', content: ' ' }, emote('EZ')];

// Rust stamps has_words with the rule engine's verdict; the row only reads it.
function render(segments: unknown[], design: Record<string, unknown>, hasWords: boolean): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, compact_emote_tooltips: true, ...design };
  const message = {
    id: 'ew',
    user_id: '1',
    username: 'viewer',
    display_name: 'Viewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '1000',
    content: 'unused',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'ew' },
    segments,
    metadata: {
      is_action: false,
      is_mentioned: false,
      is_first_message: false,
      is_from_shared_chat: false,
      shown_name: 'Viewer',
      rules_evaluated: true,
      has_words: hasWords || undefined,
    },
  };
  return renderToStaticMarkup(createElement(ChatMessage, { message: message as never }));
}

const words = (html: string) => [...html.matchAll(/class="sn-emote-word-(?:label|after)"[^>]*>([^<]*)</g)].map((m) => m[1]);

describe('emote words', () => {
  it('stays off by default', () => {
    expect(words(render(SENTENCE, {}, true))).toEqual([]);
  });

  it('puts the typed word under the emote in a sentence', () => {
    const html = render(SENTENCE, { emote_words: 'under' }, true);
    expect(html).toContain('sn-emote-word-under');
    expect(words(html)).toEqual(['EZ']);
  });

  it('draws the word after the emote, and a chip around both', () => {
    expect(words(render(SENTENCE, { emote_words: 'after' }, true))).toEqual(['EZ']);
    const chips = render(SENTENCE, { emote_words: 'chips' }, true);
    expect(chips).toContain('sn-emote-word-chip');
    expect(words(chips)).toEqual(['EZ']);
  });

  it('leaves an emote-only message bare while Sentences only is on', () => {
    expect(words(render(EMOTES_ONLY, { emote_words: 'under' }, false))).toEqual([]);
    expect(words(render(EMOTES_ONLY, { emote_words: 'under', emote_words_sentences_only: false }, false))).toEqual(['EZ', 'EZ']);
  });

  it('shows no word for a personal emote already drawn as its text', () => {
    const personal = [{ type: 'text', content: 'hi ' }, { ...emote('EZ'), is_personal: true }];
    expect(words(render(personal, { emote_words: 'under', show_personal_emotes: false }, true))).toEqual([]);
  });
});
