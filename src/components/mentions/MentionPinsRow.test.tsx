// @vitest-environment node
// SN-284 D14/D15: a pin is the real row of the chat it was said in. From an
// empty store, for a chat this window never joined, it draws the badge art and
// the emote Rust carried; it is hosted outside the chat list (data-pin-id,
// never data-message-id), never flashes and shows its time with the day.
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
vi.mock('../ProviderLogo', () => ({ ProviderLogo: () => null }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: class {}, getCurrentWebviewWindow: () => ({ label: 'main' }) }));

import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { MentionPinsBlock } from './MentionPins';
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { samplePin } from '../settings/pinSamples';
import type { MentionPin } from '../../types';

const BADGE_2X = 'https://static-cdn.jtvnw.net/badges/v1/sub12/2';
const EMOTE = 'https://cdn.7tv.app/emote/abc/2x.webp';

/** A pin from another chat, with badge art and an emote Rust carried. */
function otherChatPin(): MentionPin {
  const base = samplePin(1, 'mention', 'pixelfox', '#1E90FF', '@you Kappa hello', '9:45 PM');
  return {
    ...base,
    day: true,
    channel_mark: { room: 'kick:otherchan', platform: 'kick', channel: 'otherchan' },
    entry: { ...base.entry, at_ms: new Date(2026, 10, 19, 21, 45).getTime(), room: 'kick:otherchan', platform: 'kick' },
    row: {
      ...base.row,
      provider: 'twitch',
      badges_resolved: true,
      badges: [
        {
          name: 'subscriber',
          version: '12',
          image_url_1x: 'https://static-cdn.jtvnw.net/badges/v1/sub12/1',
          image_url_2x: BADGE_2X,
          image_url_4x: 'https://static-cdn.jtvnw.net/badges/v1/sub12/4',
          title: '1-Year Subscriber',
        },
      ],
      tags: { ...base.row.tags, badges: 'subscriber/12', 'room-id': '999' },
      segments: [
        { type: 'text', content: '@you ' },
        { type: 'emote', content: 'Kappa', emote_id: 'abc', emote_url: EMOTE },
        { type: 'text', content: ' hello' },
      ],
    },
  };
}

function html(pin: MentionPin): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, font_size: 14, show_badges: true, mention_animation: true };
  return renderToStaticMarkup(
    createElement(MentionPinsBlock, {
      pins: [pin],
      form: 'full',
      shown: false,
      warm: false,
      onOpenOwn: () => {},
      onOpenOther: () => {},
    }),
  );
}

describe('a pinned row', () => {
  it('draws the badge art and the emote it carries, from an empty chatter store', () => {
    expect(useChatUserStore.getState().users.size).toBe(0);
    const out = html(otherChatPin());
    expect(out).toContain(BADGE_2X.replace('/2', ''));
    expect(out).toContain('1-Year Subscriber');
    expect(out).toContain(EMOTE.replace('/2x.webp', ''));
  });

  it('is hosted by pin id only, never by message id', () => {
    const out = html(otherChatPin());
    expect(out).toContain('data-pin-id="sample-pin-1"');
    expect(out).not.toContain('data-message-id');
  });

  it('marks the other chat, and shows the time with its day', () => {
    const out = html(otherChatPin());
    expect(out).toContain('otherchan');
    expect(out).toMatch(/Thu|Wed|Fri|Nov 19/);
    expect(out).toContain('9:45 PM');
  });

  it('never flashes or loads link previews', () => {
    const out = html(otherChatPin());
    expect(out).not.toContain('animate-mention-flash');
    expect(out).not.toContain('animate-reply-flash');
  });

  it('draws the Clear all control only for two or more pins, on the count line', () => {
    const one = html(otherChatPin());
    expect(one).not.toContain('Clear all');
    const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
    initial.settings.chat_design = { link_previews: false, font_size: 14 };
    const two = renderToStaticMarkup(
      createElement(MentionPinsBlock, {
        pins: [samplePin(1, 'mention', 'a', '#fff', 'one', '9:00 PM'), samplePin(2, 'reply', 'b', '#fff', 'two', '9:01 PM')],
        form: 'full',
        shown: false,
        warm: false,
        onOpenOwn: () => {},
        onOpenOther: () => {},
      }),
    );
    expect(two).toContain('Clear all');
    expect(two).toContain('2 pinned mentions');
  });
});
