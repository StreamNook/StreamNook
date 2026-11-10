// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DONE_HOLD_MS,
  LANDING_MS,
  acceptsComposerEvent,
  changeDraftLater,
  forgetAccount,
  fromInfo,
  infoEdits,
  landedInfo,
  landingAfterSave,
  landingExpired,
  lastPage,
  managesHere,
  rememberPage,
  shownInfo,
  type InfoHeld,
  chatModeAction,
  clampPoints,
  codePoints,
  collapseTarget,
  freshBuilder,
  getTool,
  groupPages,
  growChoices,
  initialPage,
  isShowing,
  liveFromRoom,
  liveFromView,
  liveHeld,
  modeMatches,
  moveChoice,
  mustCloseSurfaces,
  raidStart,
  resetManagerStore,
  runTool,
  setToolDraft,
  shownMode,
  switchKind,
  toolKey,
  withHeld,
  type BuilderChoice,
  type LeavingRow,
  type ToolEntry,
} from './managerStore';
import type { PageRow, QueueGroup, QueueRow, RaidTarget, StreamInfoView } from '../../services/streamManager';

// The stream manager's per-window store: one write per key at a time, results
// that outlive the form, drafts kept unless a write cleared exactly what it
// sent, and the pure rules the forms draw with.

const K = toolKey('chan-a', 'info');
const ok = (message = 'Saved') => () => Promise.resolve({ ok: true, message });

beforeEach(() => {
  vi.useFakeTimers();
  resetManagerStore();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('runTool', () => {
  it('refuses a second run while the key is working, so a press never sends twice', async () => {
    let finish: (v: { ok: boolean; message: string }) => void = () => {};
    const first = runTool(K, () => new Promise((r) => (finish = r)));
    const run = vi.fn(ok());
    await expect(runTool(K, run)).resolves.toBe(false);
    expect(run).not.toHaveBeenCalled();
    finish({ ok: true, message: 'Saved' });
    await expect(first).resolves.toBe(true);
  });

  it('keeps done after the form is gone, then goes back to idle on its own', async () => {
    await runTool(K, ok('Stream info saved'));
    expect(getTool(K).phase).toBe('done');
    expect(getTool(K).line).toBe('Stream info saved');
    vi.advanceTimersByTime(DONE_HOLD_MS - 1);
    expect(getTool(K).phase).toBe('done');
    vi.advanceTimersByTime(1);
    expect(getTool(K).phase).toBe('idle');
    expect(getTool(K).line).toBeNull();
  });

  it("an older write's timer never ends a newer write's done", async () => {
    await runTool(K, ok('first'));
    vi.advanceTimersByTime(DONE_HOLD_MS - 10);
    await runTool(K, ok('second'));
    vi.advanceTimersByTime(20);
    expect(getTool(K).phase).toBe('done');
    expect(getTool(K).line).toBe('second');
  });

  it("keeps the draft and Rust's line when Twitch refuses", async () => {
    setToolDraft(K, { title: 'mine' });
    await expect(runTool(K, () => Promise.resolve({ ok: false, message: 'Twitch said no' }), { clearDraft: true })).resolves.toBe(false);
    expect(getTool(K)).toMatchObject({ phase: 'failed', line: 'Twitch said no', draft: { title: 'mine' } });
    vi.advanceTimersByTime(DONE_HOLD_MS * 5);
    expect(getTool(K).phase).toBe('failed');
  });

  it('words a thrown answer: its own string, else that Twitch did not answer', async () => {
    await runTool(K, () => Promise.reject('Sign in to Twitch first'));
    expect(getTool(K).line).toBe('Sign in to Twitch first');
    await runTool(K, () => Promise.reject(new Error('boom')));
    expect(getTool(K).line).toBe('Twitch did not answer.');
  });

  it('clears the draft after a success only if it is still the one sent', async () => {
    const sent = { title: 'a' };
    setToolDraft(K, sent);
    await runTool(K, ok(), { clearDraft: true });
    expect(getTool(K).draft).toBeUndefined();

    setToolDraft(K, { title: 'b' });
    let finish: (v: { ok: boolean; message: string }) => void = () => {};
    const p = runTool(K, () => new Promise((r) => (finish = r)), { clearDraft: true });
    const typedMeanwhile = { title: 'b, then more' };
    setToolDraft(K, typedMeanwhile);
    finish({ ok: true, message: 'Saved' });
    await p;
    expect(getTool(K).draft).toBe(typedMeanwhile);
  });

  it('an edit after a failure takes the key back to idle', async () => {
    await runTool(K, () => Promise.resolve({ ok: false, message: 'no' }));
    setToolDraft(K, { title: 'again' });
    expect(getTool(K)).toMatchObject({ phase: 'idle', line: null });
  });

  it('keys are per channel: a write for one never shows on another', async () => {
    await runTool(toolKey('chan-a', 'raid'), ok('Raiding X'));
    expect(getTool(toolKey('chan-b', 'raid')).phase).toBe('idle');
  });

  it('a per-redemption key is dropped once its result has shown', async () => {
    const key = toolKey('chan-a', 'rewards:r1');
    await runTool(key, ok('Completed'), { draft: { fulfill: true }, forget: true });
    expect(isShowing(getTool(key))).toBe(true);
    vi.advanceTimersByTime(DONE_HOLD_MS);
    expect(getTool(key)).toMatchObject({ phase: 'idle', gen: 0, draft: undefined });
  });

  it('a poll row ended here stays held while its result shows', async () => {
    const v = 0;
    await runTool(toolKey('chan-a', 'live:poll'), ok('Poll ended'));
    expect(liveHeld(v, 'chan-a', 'poll')).toBe(true);
    expect(liveHeld(v, 'chan-b', 'poll')).toBe(false);
    vi.advanceTimersByTime(DONE_HOLD_MS);
    expect(liveHeld(v, 'chan-a', 'poll')).toBe(false);
  });
});

describe('the account', () => {
  it('a switch forgets every draft, result, timer and remembered page', async () => {
    setToolDraft(K, { title: 'typed as the last account' });
    await runTool(toolKey('chan-a', 'raid'), ok('Raiding X'));
    rememberPage('chan-a', 'raid');
    forgetAccount();
    expect(getTool(K).draft).toBeUndefined();
    expect(getTool(toolKey('chan-a', 'raid')).phase).toBe('idle');
    expect(lastPage('chan-a')).toBeNull();
  });

  it("an old write's timer never ends a write made after the store was emptied", async () => {
    await runTool(K, ok('first'));
    const firstGen = getTool(K).gen;
    forgetAccount();
    await runTool(K, ok('second'));
    // One counter for the window, never reset: the new write is numbered past the old.
    expect(getTool(K).gen).toBeGreaterThan(firstGen);
  });
});

describe('who manages here', () => {
  const base = { twitch: true, signedIn: true, replay: false, modBadge: false, broadcaster: false, pages: null as number | null };

  it('a badge says yes at once; an editor waits for Rust, then its pages decide', () => {
    expect(managesHere({ ...base, modBadge: true })).toBe('yes');
    expect(managesHere({ ...base, broadcaster: true })).toBe('yes');
    expect(managesHere(base)).toBe('unknown');
    expect(managesHere({ ...base, pages: 4 })).toBe('yes');
    expect(managesHere({ ...base, pages: 0 })).toBe('no');
  });

  it('never off Twitch, signed out, or for a non-moderator on a replay', () => {
    expect(managesHere({ ...base, twitch: false, modBadge: true })).toBe('no');
    expect(managesHere({ ...base, signedIn: false, pages: 4 })).toBe('no');
    expect(managesHere({ ...base, replay: true, pages: 4 })).toBe('no');
    expect(managesHere({ ...base, replay: true, modBadge: true })).toBe('yes');
  });
});

describe('surfaces', () => {
  const was = { channelId: 'a', account: 'u1' };
  it('a channel switch closes them, open or not, and so does being known not to manage', () => {
    expect(mustCloseSurfaces(was, { channelId: 'b', account: 'u1', manages: 'yes' }, false)).toBe(true);
    expect(mustCloseSurfaces(was, { channelId: 'a', account: 'u1', manages: 'no' }, true)).toBe(true);
    expect(mustCloseSurfaces(was, { channelId: 'a', account: 'u1', manages: 'no' }, false)).toBe(false);
    expect(mustCloseSurfaces(was, { channelId: 'a', account: 'u1', manages: 'yes' }, true)).toBe(false);
  });

  it('nothing closes while it is still unknown whether you manage here', () => {
    expect(mustCloseSurfaces(was, { channelId: 'a', account: 'u1', manages: 'unknown' }, true)).toBe(false);
  });

  it('another Twitch account closes them; the account becoming known does not', () => {
    expect(mustCloseSurfaces(was, { channelId: 'a', account: 'u2', manages: 'yes' }, true)).toBe(true);
    expect(mustCloseSurfaces({ channelId: 'a', account: null }, { channelId: 'a', account: 'u1', manages: 'yes' }, true)).toBe(false);
    expect(mustCloseSurfaces(was, { channelId: 'a', account: null, manages: 'yes' }, true)).toBe(false);
  });
});

describe('slash command events', () => {
  it('only the pane the command was typed in, while it shows that channel', () => {
    expect(acceptsComposerEvent({ source: 'p1', channel: 'Chan', kind: 'poll' }, 'p1', 'chan')).toBe(true);
    expect(acceptsComposerEvent({ source: 'p1', channel: 'chan' }, 'p2', 'chan')).toBe(false);
    expect(acceptsComposerEvent({ source: 'p1', channel: 'other' }, 'p1', 'chan')).toBe(false);
    // A pane that does not compare rooms (the phone's) goes by its source.
    expect(acceptsComposerEvent({ source: 'p1', channel: 'any' }, 'p1', null)).toBe(true);
  });

  it('an event with no source goes to the pane showing its channel only', () => {
    expect(acceptsComposerEvent({ channel: 'chan' }, 'p1', 'chan')).toBe(true);
    expect(acceptsComposerEvent({ channel: 'chan' }, 'p1', 'other')).toBe(false);
    expect(acceptsComposerEvent({}, 'p1', 'chan')).toBe(false);
    expect(acceptsComposerEvent(undefined, 'p1', 'chan')).toBe(false);
  });
});

describe('stream info edits', () => {
  const cat = (id: string) => ({ id, name: id, box_art_url: '' });
  const info = (title: string, category = cat('1')) =>
    ({ title, category, tags: ['a'], language: 'en' }) as unknown as StreamInfoView;
  const twitch = fromInfo(info('Old title'));
  const empty: InfoHeld = { edits: {}, tagText: '', landing: {} };

  it('keeps only the fields that differ from what shows under them', () => {
    expect(infoEdits({ ...twitch, title: 'New' }, twitch)).toEqual({ title: 'New' });
    expect(infoEdits(twitch, twitch)).toEqual({});
    expect(infoEdits({ ...twitch, category: { ...cat('1'), name: 'renamed' } }, twitch)).toEqual({});
    expect(infoEdits({ ...twitch, tags: ['a', 'b'] }, twitch)).toEqual({ tags: ['a', 'b'] });
  });

  it('a saved field shows as sent until Twitch has it, and a field typed since stays an edit', () => {
    const held: InfoHeld = { ...empty, edits: { title: 'Typed after Save', category: cat('2') } };
    const after = landingAfterSave(held, { title: 'Sent', category: cat('2') }, 7);
    expect(after.edits).toEqual({ title: 'Typed after Save' });
    expect(after.landing.title).toEqual({ value: 'Sent', stamp: 7 });
    expect(after.landing.category?.value).toEqual(cat('2'));
    // Twitch's read still has the old title: the sent one shows under the edit.
    expect(shownInfo(info('Old title'), { ...after, edits: {} }).title).toBe('Sent');
    // Another window's save of the title shows; the category the read does not have yet stays.
    const landed = landedInfo(after, info('Sent'));
    expect(landed.landing.title).toBeUndefined();
    expect(landed.landing.category).toBeDefined();
    expect(landedInfo(landed, info('Sent'))).toBe(landed);
  });

  it("a save's timer drops only its own fields", () => {
    const one = landingAfterSave(empty, { title: 'A', tags: ['x'] }, 1);
    const two = landingAfterSave(one, { title: 'B' }, 2);
    const expired = landingExpired(two, 1);
    expect(expired.landing.title).toEqual({ value: 'B', stamp: 2 });
    expect(expired.landing.tags).toBeUndefined();
    expect(landingExpired(expired, 1)).toBe(expired);
  });

  it('the timer runs whether or not the form is open, and only on its own name', () => {
    setToolDraft(K, landingAfterSave(empty, { title: 'A' }, 3));
    changeDraftLater(K, `${K}:landing:3`, LANDING_MS, (d) => landingExpired(d as InfoHeld, 3));
    vi.advanceTimersByTime(LANDING_MS - 1);
    expect((getTool(K).draft as InfoHeld).landing.title).toBeDefined();
    vi.advanceTimersByTime(1);
    expect((getTool(K).draft as InfoHeld).landing.title).toBeUndefined();
  });
});

describe('pages', () => {
  const pages = (ids: string[]): PageRow[] => ids.map((id) => ({ id, group: 'Channel', label: id, live: null, badge: null }) as PageRow);

  it('opens on the page asked for, else the last one used, else the first', () => {
    expect(initialPage('raid', 'info', pages(['info', 'raid']))).toBe('raid');
    expect(initialPage(null, 'raid', pages(['info', 'raid']))).toBe('raid');
    expect(initialPage(null, null, pages(['info', 'raid']))).toBe('info');
  });

  it('never opens on a page the role no longer has', () => {
    expect(initialPage('raid', 'ads', pages(['poll', 'rewards']))).toBe('poll');
    expect(initialPage('raid', null, [])).toBeNull();
  });

  it('collapses to the panel or builder, never to Chat modes, Moderation or a running poll', () => {
    const idle = { poll: null, prediction: null };
    expect(collapseTarget('info', idle)).toBe('info');
    expect(collapseTarget('poll', idle)).toBe('poll');
    expect(collapseTarget('poll', { poll: {}, prediction: null })).toBeNull();
    expect(collapseTarget('chat_modes', idle)).toBeNull();
    expect(collapseTarget('moderation', idle)).toBeNull();
  });

  it('groups the side list under its headings in order', () => {
    const rows = [
      { id: 'info', group: 'Your channel' },
      { id: 'poll', group: 'Your channel' },
      { id: 'announce', group: 'Chat' },
    ] as PageRow[];
    expect(groupPages(rows).map((g) => [g.group, g.rows.map((r) => r.id)])).toEqual([
      ['Your channel', ['info', 'poll']],
      ['Chat', ['announce']],
    ]);
  });
});

describe('counting', () => {
  it('counts code points, as Rust does: an emoji is one', () => {
    expect(codePoints('🔴 live')).toBe(6);
    expect('🔴 live'.length).toBe(7);
    expect(codePoints('👨‍👩‍👧')).toBe(5);
    expect(codePoints('日本語')).toBe(3);
    expect(clampPoints('🔴🔴🔴', 2)).toBe('🔴🔴');
  });
});

describe('the builder', () => {
  const box = (text: string): BuilderChoice => ({ id: text || 'empty', text });

  it('grows a new box once the last has text, up to the maximum Rust sends', () => {
    expect(growChoices([box('a'), box('')], 3)).toHaveLength(2);
    expect(growChoices([box('a'), box('b')], 3)).toHaveLength(3);
    expect(growChoices([box('a'), box('b'), box('c')], 3)).toHaveLength(3);
  });

  it('switching kind drops empty boxes first, pads to the minimum and moves the length into range', () => {
    const d = { ...freshBuilder('prediction'), seconds: 15, choices: [box('a'), box(''), box('b'), box('c'), box('d'), box('e')] };
    const poll = switchKind(d, 'poll', { max: 5, min: 2, minSecs: 15, maxSecs: 1800 });
    expect(poll.choices.map((c) => c.text)).toEqual(['a', 'b', 'c', 'd', 'e']);
    const back = switchKind({ ...poll, choices: [box('x')] }, 'prediction', { max: 10, min: 2, minSecs: 30, maxSecs: 1800 });
    expect(back.choices).toHaveLength(2);
    expect(back.seconds).toBe(30);
  });

  it('the same switch shown twice gives the same boxes, so a render never re-keys them', () => {
    const one = { ...freshBuilder('poll'), choices: [box('x')] };
    const lim = { max: 10, min: 3, minSecs: 30, maxSecs: 1800 };
    expect(switchKind(one, 'prediction', lim).choices.map((c) => c.id)).toEqual(switchKind(one, 'prediction', lim).choices.map((c) => c.id));
    const ids = switchKind(one, 'prediction', lim).choices.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('moves a choice with Alt+Up/Down, and not past either end', () => {
    const c = [box('a'), box('b'), box('c')];
    expect(moveChoice(c, 1, -1).map((x) => x.text)).toEqual(['b', 'a', 'c']);
    expect(moveChoice(c, 0, -1)).toBe(c);
  });
});

describe('raid', () => {
  const live: RaidTarget = { id: '1', login: 'x', name: 'X', avatar_url: null, game_name: 'G', live: true, line: 'G' };

  it('picking sends nothing; Start raid sends the raid; a held Enter does not start it again', () => {
    expect(raidStart(null, false)).toBeNull();
    expect(raidStart(live, false)).toEqual({ kind: 'raid', target_id: '1', target_login: 'x', target_name: 'X' });
    expect(raidStart(live, true)).toBeNull();
    expect(raidStart({ ...live, live: false }, false)).toBeNull();
  });
});

describe('chat modes', () => {
  it('reads the room: followers -1 is off, 0 is any follower', () => {
    const room = liveFromRoom({ followersOnly: 0, slow: 0, subsOnly: true, emoteOnly: false, r9k: false });
    expect(room.followers).toEqual({ on: true, value: 0 });
    expect(room.slow).toEqual({ on: false, value: null });
    expect(liveFromRoom({ followersOnly: -1, slow: 30, subsOnly: false, emoteOnly: false, r9k: false }).followers.on).toBe(false);
    expect(liveFromView([{ kind: 'slow', on: true, value: 10 }]).slow).toEqual({ on: true, value: 10 });
  });

  it("turning off sends no length; turning on without one leaves it to Rust's default", () => {
    expect(chatModeAction('slow', false, 30)).toEqual({ kind: 'chat_mode', mode: 'slow', on: false, value: null });
    expect(chatModeAction('slow', true)).toEqual({ kind: 'chat_mode', mode: 'slow', on: true, value: null });
    expect(chatModeAction('followers', true, 10)).toEqual({ kind: 'chat_mode', mode: 'followers', on: true, value: 10 });
  });

  const entry = (phase: ToolEntry['phase'], draft: unknown): ToolEntry => ({ phase, line: null, gen: 1, draft, sent: undefined });
  const off = { on: false, value: null };
  const on30 = { on: true, value: 30 };
  const wanted = { on: true, value: null, roomAtPress: off };

  it('shows what was asked for while it runs, the room once it matches, the room after a refusal', () => {
    expect(shownMode(entry('working', wanted), off, off)).toEqual(wanted);
    expect(shownMode(entry('done', wanted), on30, off)).toEqual(on30);
    expect(shownMode(entry('failed', wanted), off, off)).toEqual(off);
    expect(modeMatches({ on: true, value: 10 }, on30)).toBe(false);
  });

  it("while the room's line lags, trusts Rust's read after the write over the old room, long after the hold", () => {
    expect(shownMode(entry('done', wanted), off, on30)).toEqual(on30);
    expect(shownMode(entry('idle', wanted), off, on30)).toEqual(on30);
    // No read yet: what was asked for.
    expect(shownMode(entry('idle', wanted), off, null)).toEqual(wanted);
  });

  it('once the room moves on its own, shows the room', () => {
    const someoneElse = { on: true, value: 5 };
    expect(shownMode(entry('idle', { on: false, value: null, roomAtPress: on30 }), someoneElse, off)).toEqual(someoneElse);
  });
});

describe('rewards', () => {
  const row = (id: string): QueueRow => ({ id, user_login: id, user_name: id, avatar_url: null, input: null, at_ms: 0, line: id });
  const group = (id: string, rows: QueueRow[]): QueueGroup => ({
    reward_id: id,
    title: id,
    cost: 1,
    paused: false,
    enabled: true,
    image_url: null,
    color: null,
    heading: id,
    rows,
  });

  it('keeps a row acted on in its place while its result shows, even after the queue drops it', () => {
    const g = group('r1', [row('a'), row('b'), row('c')]);
    const leaving: LeavingRow[] = [{ row: row('b'), group: g, index: 1, key: 'c:rewards:b' }];
    const after = [group('r1', [row('a'), row('c')])];
    expect(withHeld(after, leaving, () => true)[0].rows.map((r) => r.id)).toEqual(['a', 'b', 'c']);
    expect(withHeld(after, leaving, () => false)[0].rows.map((r) => r.id)).toEqual(['a', 'c']);
  });

  it("brings back a reward whose rows all left (Complete all), held by its reward's key", () => {
    const g = group('r2', [row('y'), row('z')]);
    const leaving: LeavingRow[] = [
      { row: row('y'), group: g, index: 0, key: 'c:rewards:all:r2' },
      { row: row('z'), group: g, index: 1, key: 'c:rewards:all:r2' },
    ];
    const out = withHeld([], leaving, (l) => l.key === 'c:rewards:all:r2');
    expect(out.map((x) => [x.reward_id, x.rows.map((r) => r.id)])).toEqual([['r2', ['y', 'z']]]);
  });
});
