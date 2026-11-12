import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The account's smiley set reaches a window as an event from Rust. The window
// applies Rust's swap to the Twitch emote sets it holds and to every Twitch set
// it stores afterwards, so a fetch that was already running cannot bring the
// old faces back.

const h = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  listenFails: false,
  fetchAll: vi.fn(),
  fetchKick: vi.fn(),
  invoke: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => h.invoke(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (event: { payload: unknown }) => void) => {
    if (h.listenFails) throw new Error('no event bridge');
    h.handlers.set(name, cb);
    return () => {};
  },
  emit: vi.fn(),
}));
vi.mock('./AppStore', () => ({
  useAppStore: {
    getState: () => ({ settings: {}, currentUser: null }),
    setState: vi.fn(),
    subscribe: () => () => {},
  },
}));
vi.mock('../services/emoteService', () => ({
  fetchAllEmotes: h.fetchAll,
  fetchKickChannelEmotes: h.fetchKick,
  fetchYouTubeChannelEmotes: vi.fn(async () => null),
  enhanceRustEmotes: vi.fn((x: unknown) => x),
  inlineEmoteTier: vi.fn(() => '2x'),
}));
vi.mock('../services/twitchBadges', () => ({
  parseBadges: vi.fn(() => []),
  initializeBadgeCache: vi.fn(async () => {}),
}));
vi.mock('../services/badgeImageCacheService', () => ({
  initializeBadgeImageCache: vi.fn(async () => {}),
}));
vi.mock('../utils/platform', () => ({ IS_MOBILE: true }));

const CHANNEL = 'smileyroom';

function row(id: string, name: string, extra: Record<string, unknown> = {}) {
  return { id, name, url: `https://cdn/${id}`, provider: 'twitch' as const, ...extra };
}
const robotSmile = row('1', ':)', { emote_type: 'smilies' });
const robotHeart = row('9', '<3', { emote_type: 'smilies' });
const monkeySmile = row('499', ':)', { emote_type: 'smilies' });
const monkeyHeart = row('483', '<3', { emote_type: 'smilies' });
const kappa = row('25', 'Kappa', { emote_type: 'globals' });
const MONKEYS = { remove_ids: ['1', '9'], rows: [monkeySmile, monkeyHeart] };

function twitchSet(...twitch: ReturnType<typeof row>[]) {
  return { twitch, bttv: [], '7tv': [], ffz: [], kick: [], youtube: [] };
}
const ids = (set: { twitch: { id: string }[] } | null) => (set ? set.twitch.map((e) => e.id) : null);

const event = (rev: number, swap: unknown, extra: Record<string, unknown> = {}) => ({
  rev,
  swap,
  view: null,
  shown: true,
  ...extra,
});
const fire = (payload: unknown) => h.handlers.get('twitch://smilies-changed')?.({ payload });

async function settle() {
  await vi.advanceTimersByTimeAsync(0);
}

/** A fresh copy of the store, with the listener in place and Rust's catch-up answer ready. */
async function load(catchUp: unknown = null) {
  h.invoke.mockImplementation(async (command: string) => (command === 'get_own_smilies' ? catchUp : undefined));
  const store = await import('./chatConnectionStore');
  const needs = await import('../utils/settingNeeds');
  await settle();
  return { store, needs };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetModules();
  h.handlers.clear();
  h.listenFails = false;
  h.fetchAll.mockReset();
  h.fetchKick.mockReset();
  h.invoke.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the account smiley set in this window', () => {
  it('swaps the faces of a cached Twitch set and tells its subscribers', async () => {
    const { store } = await load();
    const ffzRow = { id: '1', name: 'Zoom', url: 'https://cdn/z', provider: 'ffz' as const };
    h.fetchAll.mockResolvedValue({ ...twitchSet(robotSmile, kappa, robotHeart), ffz: [ffzRow] });
    await store.ensureChannelEmotes(CHANNEL, '1');
    const notified = vi.fn();
    store.subscribeChannelEmotes(CHANNEL, notified);

    fire(event(1, MONKEYS));

    const held = store.getChannelEmotes(CHANNEL);
    expect(ids(held)).toEqual(['25', '499', '483']);
    expect(held?.ffz).toEqual([ffzRow]);
    expect(notified).toHaveBeenCalledTimes(1);
  });

  it('does not add a face the set already holds', async () => {
    const { store } = await load();
    h.fetchAll.mockResolvedValue(twitchSet(monkeySmile, robotSmile, kappa));
    await store.ensureChannelEmotes(CHANNEL, '1');

    fire(event(1, MONKEYS));

    expect(ids(store.getChannelEmotes(CHANNEL))).toEqual(['499', '25', '483']);
  });

  it('leaves other platforms and sets without faces alone', async () => {
    const { store } = await load();
    h.fetchKick.mockResolvedValue(twitchSet(robotSmile));
    await store.ensureChannelEmotes('xqc', '', 'kick');
    const kickSet = store.getChannelEmotes('xqc', 'kick');
    h.fetchAll.mockResolvedValue(twitchSet(kappa));
    await store.ensureChannelEmotes(CHANNEL, '1');
    const plain = store.getChannelEmotes(CHANNEL);
    const notified = vi.fn();
    store.subscribeChannelEmotes(CHANNEL, notified);

    fire(event(1, MONKEYS));

    expect(store.getChannelEmotes('xqc', 'kick')).toBe(kickSet);
    expect(store.getChannelEmotes(CHANNEL)).toBe(plain);
    expect(notified).not.toHaveBeenCalled();
  });

  it('ignores an older or repeated rev', async () => {
    const { store } = await load();
    fire(event(2, MONKEYS));
    fire(event(1, null));
    fire(event(2, null));

    h.fetchAll.mockResolvedValue(twitchSet(robotSmile, kappa));
    const set = await store.ensureChannelEmotes(CHANNEL, '1');

    expect(ids(set)).toEqual(['25', '499', '483']);
  });

  it('stores the new faces for a fetch that finishes after the event', async () => {
    const { store } = await load();
    let finish: (set: unknown) => void = () => {};
    h.fetchAll.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const pending = store.ensureChannelEmotes(CHANNEL, '1');

    fire(event(1, MONKEYS));
    finish(twitchSet(robotSmile, kappa));
    const set = await pending;

    expect(ids(set)).toEqual(['25', '499', '483']);
    expect(ids(store.getChannelEmotes(CHANNEL))).toEqual(['25', '499', '483']);
  });

  it('gives a refreshed set the new faces too', async () => {
    const { store } = await load();
    fire(event(1, MONKEYS));
    h.fetchAll.mockResolvedValue(twitchSet(robotSmile, robotHeart, kappa));

    const set = await store.refreshChannelEmotes(CHANNEL, '1');

    expect(ids(set)).toEqual(['25', '499', '483']);
  });

  it('stops swapping when Rust sends no swap', async () => {
    const { store } = await load();
    fire(event(1, MONKEYS));
    fire(event(2, null));
    h.fetchAll.mockResolvedValue(twitchSet(robotSmile, kappa, robotHeart));

    const set = await store.ensureChannelEmotes(CHANNEL, '1');

    expect(ids(set)).toEqual(['1', '25', '9']);
  });

  it('catches up on what Rust emitted before the listener existed', async () => {
    const { store, needs } = await load(event(3, MONKEYS));
    h.fetchAll.mockResolvedValue(twitchSet(robotSmile, kappa));

    const set = await store.ensureChannelEmotes(CHANNEL, '1');

    expect(h.invoke).toHaveBeenCalledWith('get_own_smilies');
    expect(ids(set)).toEqual(['25', '499', '483']);
    expect(needs.settingNeedMet('smiley_style')).toBe(true);
  });

  it('still catches up when the listener cannot start', async () => {
    h.listenFails = true;
    const { store } = await load(event(1, MONKEYS));
    h.fetchAll.mockResolvedValue(twitchSet(robotSmile, kappa));

    const set = await store.ensureChannelEmotes(CHANNEL, '1');

    expect(ids(set)).toEqual(['25', '499', '483']);
  });

  it('tells the settings search whether the row exists', async () => {
    const { needs } = await load();
    expect(needs.settingNeedMet('smiley_style')).toBe(false);

    fire(event(1, null, { shown: true }));
    expect(needs.settingNeedMet('smiley_style')).toBe(true);

    fire(event(2, null, { shown: false }));
    expect(needs.settingNeedMet('smiley_style')).toBe(false);
  });

  it('hands the newest view to subscribers once per rev', async () => {
    const { store } = await load();
    const view = { can_switch: true, current: 'robots', sets: [], upgrade_url: 'https://www.twitch.tv/settings/prime' };
    const seen = vi.fn();
    store.subscribeOwnSmilies(seen);

    fire(event(1, null, { view }));
    fire(event(1, null, { view: null }));

    expect(seen).toHaveBeenCalledTimes(1);
    expect(store.getOwnSmilies()).toEqual({ rev: 1, view });
  });
});
