// The drops store takes only newer revs, registers its listener before it
// reads, holds nothing while disabled, lets go of a listener that resolves
// after its last reader left, and lays the model's tier minutes over the
// Drops page's per-drop counters.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  calls: [] as string[],
  handler: null as null | ((e: { payload: unknown }) => void),
  unlisten: vi.fn(),
  listenImpl: null as null | (() => Promise<() => void>),
  invokeResult: null as unknown,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_name: string, cb: (e: { payload: unknown }) => void) => {
    h.calls.push('listen');
    h.handler = cb;
    if (h.listenImpl) return h.listenImpl();
    return h.unlisten;
  }),
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    h.calls.push(`invoke:${cmd}`);
    return h.invokeResult;
  }),
}));
vi.mock('../utils/logger', () => ({ Logger: { warn: vi.fn() } }));

import {
  applyWatchDrops,
  getWatchDrops,
  overlayWatchTiers,
  resetWatchDropsForTest,
  subscribeFor,
  watchDropsReaders,
  type WatchDrops,
  type WatchDropsSnapshot,
} from './watchDropsStore';

const view = (over: Partial<WatchDrops> = {}): WatchDrops => ({
  channel_id: '111',
  login: 'shroud',
  game_id: 'g1',
  game_name: 'Rust',
  lead: null,
  more: 0,
  more_art: [],
  campaigns: [],
  complete: false,
  stalled: false,
  stall_episode: 0,
  ...over,
});
const snap = (rev: number, over: Partial<WatchDrops> = {}): WatchDropsSnapshot => ({ rev, view: view(over) });
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  resetWatchDropsForTest();
  h.calls = [];
  h.handler = null;
  h.unlisten.mockClear();
  h.listenImpl = null;
  h.invokeResult = snap(1);
});

describe('watch drops store', () => {
  it('takes only a newer rev', () => {
    expect(applyWatchDrops(snap(5))).toBe(true);
    expect(applyWatchDrops(snap(5, { login: 'other' }))).toBe(false);
    expect(applyWatchDrops(snap(4))).toBe(false);
    expect(getWatchDrops()?.rev).toBe(5);
    expect(applyWatchDrops(snap(6))).toBe(true);
  });

  it('registers the listener before it reads, and a late read never beats a newer event', async () => {
    h.invokeResult = snap(2);
    const release = subscribeFor(true)(() => {});
    await settle();
    expect(h.calls).toEqual(['listen', 'invoke:get_watch_drops']);
    h.handler?.({ payload: snap(1) });
    expect(getWatchDrops()?.rev).toBe(2);
    h.handler?.({ payload: snap(3, { login: 'next' }) });
    expect(getWatchDrops()?.view.login).toBe('next');
    release();
    expect(h.unlisten).toHaveBeenCalledTimes(1);
    // With nobody following, the copy would only go stale: it goes with the listener.
    expect(getWatchDrops()).toBeNull();
  });

  it('holds nothing while disabled', async () => {
    const release = subscribeFor(false)(() => {});
    await settle();
    expect(watchDropsReaders()).toBe(0);
    expect(h.calls).toEqual([]);
    release();
  });

  it('lets go of a listener that resolves after its last reader left', async () => {
    let resolveListen: (u: () => void) => void = () => {};
    h.listenImpl = () => new Promise((r) => (resolveListen = r));
    const release = subscribeFor(true)(() => {});
    await settle();
    release();
    const late = vi.fn();
    resolveListen(late);
    await settle();
    expect(late).toHaveBeenCalledTimes(1);
    expect(h.calls).toEqual(['listen']);
  });

  it('lays the model tiers over the page counters', () => {
    const v = view({
      campaigns: [
        {
          id: 'c1', name: 'Rust Drops', game_name: 'Rust', art: '', box_art: '', ends: '3d left', claimed_label: '1 of 3 claimed',
          started: true, needs_link: false, link_url: null, cumulative_minutes: 90,
          drops: [
            { id: 'a', instance_id: null, name: 'Cap', image: '', claimed: true, ready: false, pending: false, connect: false, lead: false, status: 'Claimed', detail: null, bar: 100, current_minutes: 60, required_minutes: 60, claim_label: 'Claim' },
            { id: 'b', instance_id: null, name: 'Hoodie', image: '', claimed: false, ready: false, pending: false, connect: false, lead: true, status: '30m to watch', detail: null, bar: 75, current_minutes: 90, required_minutes: 120, claim_label: 'Claim' },
          ],
        },
        {
          id: 'c2', name: 'Not started', game_name: 'Rust', art: '', box_art: '', ends: '', claimed_label: '', started: false, needs_link: false, link_url: null, cumulative_minutes: 0,
          drops: [{ id: 'z', instance_id: null, name: 'Jacket', image: '', claimed: false, ready: false, pending: false, connect: false, lead: false, status: 'Starts when you watch', detail: null, bar: 0, current_minutes: 0, required_minutes: 60, claim_label: 'Claim' }],
        },
      ],
    });
    const page = [
      { campaign_id: 'c1', drop_id: 'a', current_minutes_watched: 0, required_minutes_watched: 60, is_claimed: true, last_updated: '' },
      { campaign_id: 'x', drop_id: 'other', current_minutes_watched: 5, required_minutes_watched: 10, is_claimed: false, last_updated: '' },
    ];
    const out = overlayWatchTiers(page, v);
    expect(out.find((p) => p.drop_id === 'a')?.current_minutes_watched).toBe(60);
    expect(out.find((p) => p.drop_id === 'b')).toMatchObject({ campaign_id: 'c1', current_minutes_watched: 90, required_minutes_watched: 120 });
    expect(out.find((p) => p.drop_id === 'other')?.current_minutes_watched).toBe(5);
    expect(out.find((p) => p.drop_id === 'z')).toBeUndefined();
    expect(overlayWatchTiers(page, view({ channel_id: '' }))).toBe(page);
  });
});
