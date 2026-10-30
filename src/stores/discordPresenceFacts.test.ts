import { describe, expect, it, vi } from 'vitest';
import type { MultiNookSlot } from '../types';
import { createSender, gridFacts, mediaFacts, type MediaInput, type SenderDeps, type WriteResult } from './discordPresenceFacts';

const slot = (id: string, extra: Partial<MultiNookSlot> = {}): MultiNookSlot => ({
  id,
  channelLogin: id,
  volume: 1,
  muted: false,
  isFocused: false,
  ...extra,
});

describe('gridFacts', () => {
  it('is null while the grid is closed or empty', () => {
    expect(gridFacts({ isMultiNookActive: false, maximizedSlotId: null, slots: [slot('a')] }, null)).toBeNull();
    expect(gridFacts({ isMultiNookActive: true, maximizedSlotId: null, slots: [] }, null)).toBeNull();
  });

  it('copies each tile as it is, with the maximized and chat-focused marks', () => {
    const facts = gridFacts(
      {
        isMultiNookActive: true,
        maximizedSlotId: 'b',
        slots: [
          slot('a', { provider: 'kick', channelName: 'A', gameName: 'Slots', loadError: true, isMinimized: true }),
          slot('b', { channelId: '42', title: 't', profileImageUrl: 'https://x/a.png', offline: true }),
        ],
      },
      'a',
    );
    expect(facts).toEqual([
      {
        provider: 'kick', login: 'a', user_id: '', name: 'A', title: '', category: 'Slots', avatar_url: '',
        maximized: false, minimized: true, chat_focused: true, offline: false, load_error: true,
      },
      {
        provider: 'twitch', login: 'b', user_id: '42', name: '', title: 't', category: '', avatar_url: 'https://x/a.png',
        maximized: true, minimized: false, chat_focused: false, offline: true, load_error: false,
      },
    ]);
  });

  it('ignores what the facts do not carry (a volume drag changes nothing)', () => {
    const base = { isMultiNookActive: true, maximizedSlotId: null };
    const before = gridFacts({ ...base, slots: [slot('a', { volume: 0.2 })] }, null);
    const after = gridFacts({ ...base, slots: [slot('a', { volume: 0.9, muted: true })] }, null);
    expect(JSON.stringify(after)).toBe(JSON.stringify(before));
  });
});

describe('mediaFacts', () => {
  const stream = { user_login: 'c', user_id: '7', user_name: 'C', title: 'T', game_name: 'G', profile_image_url: '' };
  const input = (over: Partial<MediaInput>): MediaInput => ({
    currentMediaType: 'video',
    streamUrl: 'http://127.0.0.1/x.m3u8',
    vodPlayback: { vod_id: '1' },
    originalMediaUrl: 'https://www.twitch.tv/videos/1',
    currentStream: stream,
    ...over,
  });

  it('reports clips, VODs and the offline room, nothing else', () => {
    expect(mediaFacts(input({}))?.media_type).toBe('video');
    expect(mediaFacts(input({ currentMediaType: 'clip', vodPlayback: null }))?.vod).toBe(false);
    expect(mediaFacts(input({ currentMediaType: 'live' }))).toBeNull();
    expect(mediaFacts(input({ currentMediaType: null }))).toBeNull();
    expect(mediaFacts(input({ currentMediaType: 'offline_chat' }))?.playing).toBe(true);
  });

  it('is not playing while the stream URL is gone or the room placeholder', () => {
    expect(mediaFacts(input({ streamUrl: null }))?.playing).toBe(false);
    expect(mediaFacts(input({ currentMediaType: 'offline_chat', streamUrl: 'offline' }))?.playing).toBe(false);
  });
});

function harness(results: Array<WriteResult | Error>) {
  const sent: Array<{ gen: number; seq: number; value: unknown }> = [];
  const retries: Array<() => void> = [];
  let gen = 1;
  const deps: SenderDeps = {
    send: vi.fn(async (g: number, seq: number, value: unknown) => {
      sent.push({ gen: g, seq, value });
      const next = results.shift() ?? 'applied';
      if (next instanceof Error) throw next;
      return next;
    }),
    gen: () => gen,
    refreshGen: async () => {
      gen += 1;
    },
    later: (fn) => {
      retries.push(fn);
      return () => {
        const i = retries.indexOf(fn);
        if (i >= 0) retries.splice(i, 1);
      };
    },
    now: () => 0,
  };
  return { sent, retries, deps };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe('createSender', () => {
  it('sends a value once and skips an equal one', async () => {
    const h = harness([]);
    const s = createSender<string>(h.deps);
    s.push('a');
    await settle();
    s.push('a');
    await settle();
    expect(h.sent.map((x) => x.value)).toEqual(['a']);
  });

  it('keeps one in flight and sends only the newest after it', async () => {
    const h = harness([]);
    const s = createSender<string>(h.deps);
    s.push('a');
    s.push('b');
    s.push('c');
    await settle();
    await settle();
    expect(h.sent.map((x) => x.value)).toEqual(['a', 'c']);
    expect(h.sent.map((x) => x.seq)).toEqual([1, 2]);
  });

  it('re-reads the generation on stale and resends', async () => {
    const h = harness(['stale', 'applied']);
    const s = createSender<string>(h.deps);
    s.push('a');
    await settle();
    await settle();
    expect(h.sent).toEqual([
      { gen: 1, seq: 1, value: 'a' },
      { gen: 2, seq: 2, value: 'a' },
    ]);
  });

  it('retries a failure later', async () => {
    const h = harness([new Error('down'), 'applied', 'applied']);
    const s = createSender<string>(h.deps);
    s.push('a');
    await settle();
    expect(h.retries).toHaveLength(1);
    h.retries[0]();
    await settle();
    expect(h.sent.map((x) => x.value)).toEqual(['a', 'a']);
    s.push('a');
    await settle();
    expect(h.sent).toHaveLength(2);
  });

  it('a new value after a failure is sent at once, the retry cancelled', async () => {
    const h = harness([new Error('down'), 'applied']);
    const s = createSender<string>(h.deps);
    s.push('a');
    await settle();
    expect(h.retries).toHaveLength(1);
    s.push('b');
    await settle();
    expect(h.sent.map((x) => x.value)).toEqual(['a', 'b']);
    expect(h.retries).toHaveLength(0);
  });

  it('a reset while a write is in flight still sends the value again', async () => {
    const h = harness([]);
    const s = createSender<string>(h.deps);
    s.push('a');
    s.reset();
    s.push('a');
    await settle();
    await settle();
    expect(h.sent.map((x) => x.value)).toEqual(['a', 'a']);
  });

  it('sends again after reset, even an equal value', async () => {
    const h = harness([]);
    const s = createSender<string>(h.deps);
    s.push('a');
    await settle();
    s.reset();
    s.push('a');
    await settle();
    expect(h.sent.map((x) => x.value)).toEqual(['a', 'a']);
  });
});
