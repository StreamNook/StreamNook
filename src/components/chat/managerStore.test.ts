// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CHAT_MODE_HOLD_MS,
  DONE_HOLD_MS,
  chatModeAction,
  clampPoints,
  codePoints,
  collapseTarget,
  freshBuilder,
  getTool,
  groupPages,
  growChoices,
  initialPage,
  liveFromRoom,
  liveFromView,
  modeMatches,
  moveChoice,
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
import type { PageRow, QueueGroup, QueueRow, RaidTarget } from '../../services/streamManager';

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

  it('a chat mode keeps what was asked for during its longer hold', async () => {
    const key = toolKey('chan-a', 'chat:slow');
    await runTool(key, ok('Chat updated'), { draft: { on: true, value: null }, holdMs: CHAT_MODE_HOLD_MS });
    vi.advanceTimersByTime(DONE_HOLD_MS + 1);
    expect(getTool(key).phase).toBe('done');
    expect(getTool(key).draft).toEqual({ on: true, value: null });
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

  it('shows what was asked for until the room catches up, then the room; a refusal shows the room', () => {
    const entry = (phase: ToolEntry['phase'], draft: unknown): ToolEntry => ({ phase, line: null, gen: 1, draft, base: null, sent: undefined });
    const off = { on: false, value: null };
    const wanted = { on: true, value: null };
    expect(shownMode(entry('working', wanted), off)).toEqual(wanted);
    expect(shownMode(entry('done', wanted), off)).toEqual(wanted);
    expect(shownMode(entry('done', wanted), { on: true, value: 30 })).toEqual({ on: true, value: 30 });
    expect(shownMode(entry('failed', wanted), off)).toEqual(off);
    expect(shownMode(entry('idle', wanted), off)).toEqual(off);
    expect(modeMatches({ on: true, value: 10 }, { on: true, value: 30 })).toBe(false);
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
    const leaving: LeavingRow[] = [{ row: row('b'), group: g, index: 1 }];
    const after = [group('r1', [row('a'), row('c')])];
    expect(withHeld(after, leaving, () => true)[0].rows.map((r) => r.id)).toEqual(['a', 'b', 'c']);
    expect(withHeld(after, leaving, () => false)[0].rows.map((r) => r.id)).toEqual(['a', 'c']);
  });

  it('brings back a reward whose last row left', () => {
    const g = group('r2', [row('z')]);
    const out = withHeld([], [{ row: row('z'), group: g, index: 0 }], () => true);
    expect(out.map((x) => [x.reward_id, x.rows.map((r) => r.id)])).toEqual([['r2', ['z']]]);
  });
});
