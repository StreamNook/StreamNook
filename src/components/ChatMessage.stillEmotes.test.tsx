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
import type { EmoteSet } from '../services/emoteService';
import { markNoStillFrame } from '../utils/staticEmoteUrl';
import { setLowPower, setLowPowerSource } from '../utils/lowPower';

// Still emotes (Battery Saver on a phone, or Animate emotes: Never) draw each
// emote's first frame. 7TV and BTTV publish one only for emotes that animate,
// so KEKW, EZ and FeelsBadMan, which do not, drew as broken images.
function row(segments: unknown[]) {
  return {
    id: 'still',
    user_id: '1',
    username: 'viewer',
    display_name: 'Viewer',
    color: '#FF69B4',
    badges: [],
    timestamp: '1000',
    content: 'unused',
    provider: 'twitch',
    channel: 'chan',
    tags: { 'user-id': '1', id: 'still' },
    segments,
    metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false, shown_name: 'Viewer' },
  };
}

function render(segments: unknown[], emotes: EmoteSet | null = null, animate: 'always' | 'never' = 'never'): string {
  const initial = useAppStore.getInitialState() as unknown as { settings: Record<string, unknown> };
  initial.settings.chat_design = { link_previews: false, animate_emotes: animate };
  return renderToStaticMarkup(createElement(ChatMessage, { message: row(segments) as never, emotes }));
}

const KEKW_ID = '01F61B1440000991F7SWQNMVX7';
const KEKW_URL = `https://cdn.7tv.app/emote/${KEKW_ID}/1x.webp`;
const kekw = { type: 'emote', content: 'KEKW', emote_id: KEKW_ID, emote_url: KEKW_URL };
const catJam = { type: 'emote', content: 'catJAM', emote_id: '01CATJAM0000000000000000AA', emote_url: 'https://cdn.7tv.app/emote/01CATJAM0000000000000000AA/1x.webp' };

const set = (animated: boolean | undefined): EmoteSet => ({
  twitch: [],
  bttv: [],
  '7tv': [{ id: KEKW_ID, name: 'KEKW', url: KEKW_URL, provider: '7tv', animated }],
  ffz: [],
  kick: [],
  youtube: [],
});

const imgTag = (html: string, alt: string) => html.match(/<img\b[^>]*>/g)?.find((t) => t.includes(`alt="${alt}"`)) ?? '';

describe('still emotes', () => {
  it('an emote the channel set calls still draws its normal file', () => {
    const tag = imgTag(render([kekw], set(false)), 'KEKW');
    expect(tag).toContain(`src="${KEKW_URL}"`);
    expect(tag).not.toContain('_static');
  });

  it('an emote that may animate asks for its first frame', () => {
    expect(imgTag(render([kekw], set(undefined)), 'KEKW')).toContain('_static');
    expect(imgTag(render([catJam]), 'catJAM')).toContain('_static');
  });

  it('an emote whose first frame was missing draws its normal file from then on', () => {
    markNoStillFrame('7tv', KEKW_ID);
    expect(imgTag(render([kekw]), 'KEKW')).not.toContain('_static');
  });
});

// Animate emotes: Always, so only the still flag can stop them. Quiet alone
// (Performance mode without Freeze emotes) keeps chat moving.
// Rust's wire names: the row mapper reads gif_url and gif_id.
const GIF_URL = 'https://media.giphy.com/media/sn/giphy.gif';
const gif = { type: 'gif', content: '[cat dance]', gif_url: GIF_URL, gif_id: 'sn-gif' };

describe('still follows the still flag, not quiet', () => {
  const OFF = { quiet: false, still: false, flat: false };

  it('Performance mode without Freeze emotes keeps emotes and GIFs moving', () => {
    try {
      setLowPowerSource('performance', { quiet: true, still: false, flat: true });
      const html = render([catJam, gif], null, 'always');
      expect(imgTag(html, 'catJAM')).not.toContain('_static');
      expect(html).toContain(GIF_URL);
    } finally {
      setLowPowerSource('performance', OFF);
    }
  });

  it('Freeze emotes draws first frames and GIF chips', () => {
    try {
      setLowPowerSource('performance', { quiet: true, still: true, flat: true });
      const html = render([catJam, gif], null, 'always');
      expect(imgTag(html, 'catJAM')).toContain('_static');
      expect(html).not.toContain(GIF_URL);
    } finally {
      setLowPowerSource('performance', OFF);
    }
  });

  it('Battery Saver still draws first frames and GIF chips', () => {
    try {
      setLowPower(true);
      const html = render([catJam, gif], null, 'always');
      expect(imgTag(html, 'catJAM')).toContain('_static');
      expect(html).not.toContain(GIF_URL);
    } finally {
      setLowPower(false);
    }
  });
});
