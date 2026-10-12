// Run with: npm test
//
// A merged feed (the floating chat overlay, MultiChat's blended pane, combined
// chat) must never show a message again once it has left the feed. These drive
// the real useBlendedChatSource with a fake store and a one-instance hook
// runner (refs persist between "renders" by call order), and build the slices
// with the store's own trim policy, so what the hook sees is what a live slice
// holds.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  refs: [] as Array<{ current: unknown }>,
  cursor: 0,
  historyMax: 100,
  state: {
    channels: new Map<string, { messages: unknown[]; deletedMessageIds: Set<string>; clearedUserContexts: Map<string, unknown> }>(),
    revisionByChannel: {} as Record<string, number>,
  },
}));

vi.mock('react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react')>();
  return {
    ...actual,
    useRef: (init: unknown) => {
      const i = h.cursor++;
      if (!h.refs[i]) h.refs[i] = { current: init };
      return h.refs[i];
    },
    // The hook's memo is idempotent by design (StrictMode double-invokes it), so
    // recomputing on every call is a faithful stand-in.
    useMemo: (f: () => unknown) => f(),
  };
});

vi.mock('../stores/chatConnectionStore', () => {
  const sliceLookupKey = (provider: string, channel: string) =>
    provider === 'twitch' ? channel.trim().toLowerCase() : `${provider}:${channel.trim().toLowerCase()}`;
  const useChatConnectionStore = Object.assign(
    (sel: (s: typeof h.state) => unknown) => sel(h.state),
    { getState: () => h.state },
  );
  return {
    useChatConnectionStore,
    sliceLookupKey,
    getActiveHistoryMax: () => h.historyMax,
    useSliceViewers: () => {},
  };
});
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ settings: {}, currentUser: null }) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { useBlendedChatSource } from './useBlendedChatSource';
import { trimWithEventRetention } from '../stores/chatBufferTrim';

type Msg = { id: string; timestamp: string; provider: 'twitch'; tags: Record<string, string>; metadata: Record<string, unknown> };

let clock = 1_700_000_000_000;
function msg(id: string, event = false): Msg {
  clock += 1000;
  return {
    id,
    timestamp: String(clock),
    provider: 'twitch',
    // A resub carries the chatter's own comment and is an event row, which the
    // slice keeps past its cap (event retention).
    tags: event ? { 'msg-id': 'resub', 'system-msg': 'resubscribed' } : {},
    metadata: event ? { msg_type: 'resub', system_message: 'resubscribed' } : {},
  };
}

function setSlice(channel: string, messages: unknown[]) {
  const prev = h.state.channels.get(channel);
  h.state.channels.set(channel, {
    messages,
    deletedMessageIds: prev?.deletedMessageIds ?? new Set(),
    clearedUserContexts: prev?.clearedUserContexts ?? new Map(),
  });
  h.state.revisionByChannel[channel] = (h.state.revisionByChannel[channel] ?? 0) + 1;
}

function render(channels: Array<{ channel: string; provider: 'twitch' }>): string[] {
  h.cursor = 0;
  // eslint-disable-next-line react-hooks/rules-of-hooks -- one-instance runner above, not a React render
  const { messages } = useBlendedChatSource(channels, false);
  return messages.map((m) => (typeof m === 'string' ? m : m.id));
}

/** Records every render and reports each time an id comes back after leaving. */
function reshowTracker() {
  const gone = new Set<string>();
  let shown = new Set<string>();
  const reshows: string[] = [];
  return {
    reshows,
    record(ids: string[]) {
      const now = new Set(ids);
      for (const id of now) {
        if (!gone.has(id)) continue;
        reshows.push(id);
        gone.delete(id);
      }
      for (const id of shown) if (!now.has(id)) gone.add(id);
      expect(now.size, 'no id twice in one feed').toBe(ids.length);
      shown = now;
    },
  };
}

beforeEach(() => {
  h.refs = [];
  h.cursor = 0;
  h.historyMax = 100;
  h.state.channels = new Map();
  h.state.revisionByChannel = {};
});

describe('a message that left the merged feed never comes back', () => {
  it('event rows the slice keeps past its cap are not re-appended at the bottom', () => {
    // The floating overlay: one channel, so the feed cap equals the slice cap,
    // and only the rescued event rows sit above it.
    h.historyMax = 10;
    const channels = [{ channel: 'chan', provider: 'twitch' as const }];
    const tracker = reshowTracker();
    let slice: unknown[] = [];
    let live = 0;
    for (let i = 0; i < 120; i++) {
      live += 1;
      slice = trimWithEventRetention(slice.concat([msg(`m${i}`, i % 7 === 0)]), h.historyMax, live);
      setSlice('chan', slice);
      tracker.record(render(channels));
    }
    expect(tracker.reshows).toEqual([]);
    // Still a live feed: the newest rows are on screen, the oldest are not.
    const ids = render(channels);
    expect(ids[ids.length - 1]).toBe('m119');
    expect(ids).not.toContain('m0');
    expect(ids.length).toBeLessThanOrEqual(h.historyMax);
  });

  it('the opening backlog cut by the feed cap stays cut', () => {
    // Opening tick: the slice already holds rescued events above its cap.
    h.historyMax = 4;
    const channels = [{ channel: 'chan', provider: 'twitch' as const }];
    const e1 = msg('e1', true);
    const rows = [msg('m1'), msg('m2'), msg('m3'), msg('m4')];
    setSlice('chan', [e1, ...rows]);
    expect(render(channels)).toEqual(['m1', 'm2', 'm3', 'm4']);
    // Next live row: the slice drops m1 and keeps rescuing e1.
    const m5 = msg('m5');
    setSlice('chan', [e1, rows[1], rows[2], rows[3], m5]);
    expect(render(channels)).toEqual(['m2', 'm3', 'm4', 'm5']);
  });

  it('a buffer setting above the merged ceiling does not recycle old rows', () => {
    // 450 rows per slice, merged ceiling 400: the 50 oldest are cut on open and
    // must not return on the next tick.
    h.historyMax = 450;
    const channels = [{ channel: 'chan', provider: 'twitch' as const }];
    const tracker = reshowTracker();
    let slice: unknown[] = [];
    for (let i = 0; i < 450; i++) slice.push(msg(`r${i}`));
    setSlice('chan', slice);
    tracker.record(render(channels));
    for (let i = 450; i < 470; i++) {
      slice = slice.slice(1).concat([msg(`r${i}`)]);
      setSlice('chan', slice);
      tracker.record(render(channels));
    }
    expect(tracker.reshows).toEqual([]);
    const ids = render(channels);
    expect(ids.length).toBe(400);
    expect(ids[0]).toBe('r70');
    expect(ids[ids.length - 1]).toBe('r469');
  });

  it('five blended sources at the default buffer do not recycle plain chat', () => {
    // 5 x 100 rows held, merged cap 400: every cut row is still in its slice.
    h.historyMax = 100;
    const names = ['a', 'b', 'c', 'd', 'e'];
    const channels = names.map((channel) => ({ channel, provider: 'twitch' as const }));
    const slices = new Map<string, unknown[]>(names.map((n) => [n, []]));
    const tracker = reshowTracker();
    for (let i = 0; i < 160; i++) {
      for (const n of names) {
        const next = slices.get(n)!.concat([msg(`${n}${i}`)]);
        slices.set(n, next.length > h.historyMax ? next.slice(next.length - h.historyMax) : next);
        setSlice(n, slices.get(n)!);
      }
      tracker.record(render(channels));
    }
    expect(tracker.reshows).toEqual([]);
    expect(render(channels).length).toBe(400);
  });
});
