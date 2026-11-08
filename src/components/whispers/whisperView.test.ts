import { describe, expect, it } from 'vitest';
import {
  applyMessageUpdate,
  applyRowUpdate,
  archiveOwnerOf,
  archiveSyncing,
  canRetry,
  counterText,
  cutWords,
  dayLabel,
  deliveryLabel,
  draftRow,
  filterRows,
  isOverLimit,
  liveLine,
  mergeDraft,
  mergeMessages,
  ownerChanged,
  sortRows,
  syncLine,
  syncStatusWords,
  threadLayout,
} from './whisperView';
import type { StoredWhisperMessage, WhisperDelivery, WhisperListPayload, WhisperRowMeta, WhisperUpdatePayload } from '../../types';

const msg = (over: Partial<StoredWhisperMessage> = {}): StoredWhisperMessage => ({
  id: 'm1',
  from_user_id: '99999999',
  from_user_login: 'alpha',
  from_user_name: 'Alpha',
  to_user_id: '1000000000',
  to_user_login: 'me',
  to_user_name: 'Me',
  message: 'hello there',
  timestamp: 1000,
  is_sent: false,
  parts: [],
  deleted: false,
  edited_ms: null,
  origin: 'twitch',
  delivery: null,
  failure: null,
  cut_to: null,
  ...over,
});

const mine = (delivery: WhisperDelivery | null, over: Partial<StoredWhisperMessage> = {}) =>
  msg({ id: `s-${delivery}`, is_sent: true, delivery, origin: 'local_sent', ...over });

const row = (over: Partial<WhisperRowMeta> = {}): WhisperRowMeta => ({
  ...draftRow({ id: '99999999', login: 'alpha', display_name: 'Alpha' }, 1000),
  ...over,
});

const update = (over: Partial<WhisperUpdatePayload> = {}): WhisperUpdatePayload => ({
  owner_id: '1000000000',
  key: '99999999',
  replaced_key: null,
  meta: row(),
  message: null,
  removed_message_id: null,
  loading: null,
  error: null,
  ...over,
});

const list = (conversations: WhisperRowMeta[]): WhisperListPayload => ({
  owner_id: '1000000000',
  conversations,
  unread_total: 0,
  syncing: false,
  signed_in: true,
  session_problem: null,
});

describe('where a send stands', () => {
  it('words every state Rust sets', () => {
    expect(deliveryLabel(mine('sending'))).toBe('Sending...');
    expect(deliveryLabel(mine('sent'))).toBe('Sent');
    expect(deliveryLabel(mine('unknown'))).toBe('Checking...');
    expect(deliveryLabel(mine('delivered'))).toBe('Delivered');
    expect(deliveryLabel(mine('not_delivered'))).toBe('Not delivered');
  });

  it('says nothing for their messages or a send from before tracking', () => {
    expect(deliveryLabel(msg())).toBe('');
    expect(deliveryLabel(mine(null))).toBe('');
  });

  it('reads a state it does not know as Sent', () => {
    expect(deliveryLabel(mine('queued' as unknown as WhisperDelivery))).toBe('Sent');
  });

  it('offers Retry only under a send that did not arrive', () => {
    expect(canRetry(mine('not_delivered'))).toBe(true);
    for (const d of ['sending', 'sent', 'unknown', 'delivered'] as const) expect(canRetry(mine(d))).toBe(false);
    expect(canRetry(mine(null))).toBe(false);
    expect(canRetry(msg({ delivery: 'not_delivered' }))).toBe(false);
  });

  it('says when Twitch cut a whisper', () => {
    expect(cutWords(mine('delivered', { cut_to: 500 }))).toBe('Twitch cut this whisper to 500 characters.');
    expect(cutWords(mine('delivered'))).toBeNull();
  });
});

describe('the composer limit counts characters the way Rust does', () => {
  it('counts an emoji as one character', () => {
    expect(isOverLimit('😀'.repeat(500))).toBe(false);
    expect(isOverLimit('😀'.repeat(501))).toBe(true);
  });

  it('shows the counter from 450 and ignores outer spaces', () => {
    expect(counterText('a'.repeat(449))).toBeNull();
    expect(counterText('😀'.repeat(450))).toBe('450/500');
    expect(counterText(`  ${'a'.repeat(449)}  `)).toBeNull();
    expect(counterText('a'.repeat(501))).toBe('501/500');
  });
});

describe('a pushed change lands once', () => {
  it('joins a new message in time order', () => {
    const next = applyMessageUpdate([msg({ id: 'a', timestamp: 1 }), msg({ id: 'c', timestamp: 3 })], update({ message: msg({ id: 'b', timestamp: 2 }) }));
    expect(next.map((m) => m.id)).toEqual(['a', 'b', 'c']);
  });

  it('replaces the copy with the same id', () => {
    const next = applyMessageUpdate([mine('sending', { id: 'x' })], update({ message: mine('sent', { id: 'x' }) }));
    expect(next).toHaveLength(1);
    expect(next[0].delivery).toBe('sent');
  });

  it("swaps our copy for Twitch's when Rust adopts it", () => {
    const local = mine('sent', { id: 'local-1', timestamp: 5 });
    const twitch = mine('delivered', { id: 'uuid-1', timestamp: 6, origin: 'twitch' });
    const next = applyMessageUpdate([msg({ id: 'a', timestamp: 1 }), local], update({ message: twitch, removed_message_id: 'local-1' }));
    expect(next.map((m) => m.id)).toEqual(['a', 'uuid-1']);
  });

  it('drops a discarded send', () => {
    expect(applyMessageUpdate([mine('not_delivered', { id: 'z' })], update({ removed_message_id: 'z' }))).toEqual([]);
  });

  it('merges an older page without repeats', () => {
    const merged = mergeMessages([msg({ id: 'a', timestamp: 1 }), msg({ id: 'b', timestamp: 2 })], [msg({ id: 'b', timestamp: 2 }), msg({ id: 'c', timestamp: 3 })]);
    expect(merged.map((m) => m.id)).toEqual(['a', 'b', 'c']);
  });
});

describe('the conversation list', () => {
  it('moves a changed row to the top and drops an archived one', () => {
    const start = list([row({ key: 'k1', user_id: 'k1', last_message_timestamp: 10 }), row({ key: 'k2', user_id: 'k2', last_message_timestamp: 5 })]);
    const moved = applyRowUpdate(start, update({ key: 'k2', meta: row({ key: 'k2', user_id: 'k2', last_message_timestamp: 20 }) }));
    expect(moved?.conversations.map((r) => r.key)).toEqual(['k2', 'k1']);
    const archived = applyRowUpdate(start, update({ key: 'k1', meta: row({ key: 'k1', user_id: 'k1', archived: true }) }));
    expect(archived?.conversations.map((r) => r.key)).toEqual(['k2']);
  });

  it('replaces a re-keyed row and ignores another account', () => {
    const start = list([row({ key: 'alpha', user_id: 'alpha' })]);
    const rekeyed = applyRowUpdate(start, update({ key: '99999999', replaced_key: 'alpha' }));
    expect(rekeyed?.conversations.map((r) => r.key)).toEqual(['99999999']);
    expect(applyRowUpdate(start, update({ owner_id: '42' }))).toBe(start);
  });

  it('keeps a draft only until Rust has that person', () => {
    const draft = draftRow({ id: '1000000000', login: 'beta', display_name: 'Beta' }, 50);
    expect(mergeDraft([row()], draft).map((r) => r.key)).toEqual(['1000000000', '99999999']);
    expect(mergeDraft([row({ key: '1000000000', user_id: '1000000000' })], draft)).toHaveLength(1);
  });

  it('sorts by unread, then newest', () => {
    const rows = [row({ key: 'a', unread_count: 0, last_message_timestamp: 9 }), row({ key: 'b', unread_count: 2, last_message_timestamp: 1 })];
    expect(sortRows(rows, 'unread').map((r) => r.key)).toEqual(['b', 'a']);
    expect(sortRows(rows, 'recent').map((r) => r.key)).toEqual(['a', 'b']);
  });
});

describe('what the archive event and the refresh answer say', () => {
  it('reads the owner and the sync state from either payload shape', () => {
    expect(archiveOwnerOf('1000000000')).toBe('1000000000');
    expect(archiveOwnerOf({ owner_id: '1000000000', syncing: true })).toBe('1000000000');
    expect(archiveOwnerOf(null)).toBeNull();
    expect(archiveSyncing({ owner_id: 'x', syncing: true })).toBe(true);
    expect(archiveSyncing('x')).toBe(false);
  });

  it('reads the list again when an event belongs to another account', () => {
    expect(ownerChanged('99999999', '1000000000')).toBe(true);
    expect(ownerChanged('99999999', '99999999')).toBe(false);
    // Nothing loaded yet, or an event that names nobody: nothing to compare.
    expect(ownerChanged(null, '1000000000')).toBe(false);
    expect(ownerChanged('99999999', null)).toBe(false);
  });

  it('words each refresh outcome', () => {
    expect(syncStatusWords({ outcome: 'done', added: 0 })).toBe('Up to date');
    expect(syncStatusWords({ outcome: 'done', added: 1 })).toBe('Added 1 new message');
    expect(syncStatusWords({ outcome: 'done', added: 3 })).toBe('Added 3 new messages');
    expect(syncStatusWords({ outcome: 'wrong_session' })).toBe('Sign in to twitch.tv again to load your whispers');
    expect(syncStatusWords({ outcome: 'failed', reason: 'Twitch did not answer.' })).toBe('Could not refresh: Twitch did not answer.');
    expect(syncStatusWords({ outcome: 'superseded' })).toBeNull();
  });
});

describe('the thread as drawn', () => {
  const NOW = new Date(2026, 10, 7, 21, 41).getTime();
  const MIN = 60_000;
  const at = (id: string, minutesAgo: number, over: Partial<StoredWhisperMessage> = {}) =>
    msg({ id, timestamp: NOW - minutesAgo * MIN, ...over });

  it('words the day lines', () => {
    expect(dayLabel(NOW - 5 * MIN, NOW)).toMatch(/^Today · /);
    expect(dayLabel(NOW - 24 * 60 * MIN, NOW)).toBe('Yesterday');
    expect(dayLabel(new Date(2025, 0, 3).getTime(), NOW)).toContain('2025');
  });

  it('runs one side together and starts a new run after five minutes', () => {
    const items = threadLayout([at('a', 30), at('b', 29), at('c', 20), at('d', 19, { is_sent: true })], NOW);
    const msgs = items.filter((i) => i.kind === 'message');
    expect(msgs.map((i) => (i.kind === 'message' ? [i.id, i.first, i.last] : null))).toEqual([
      ['a', true, false],
      ['b', false, true],
      ['c', true, true],
      ['d', true, true],
    ]);
    expect(items[0]).toMatchObject({ kind: 'day' });
  });

  it('adds a time line after an hour of quiet', () => {
    const items = threadLayout([at('a', 200), at('b', 100)], NOW);
    expect(items.filter((i) => i.kind === 'day')).toHaveLength(2);
  });

  it('puts the status under the newest send of a run, and always under a failed one', () => {
    const items = threadLayout(
      [
        at('s1', 10, { is_sent: true, delivery: 'not_delivered' }),
        at('s2', 9, { is_sent: true, delivery: 'delivered' }),
        at('s3', 8, { is_sent: true, delivery: 'delivered' }),
        at('t', 7),
      ],
      NOW,
    );
    const status = items.flatMap((i) => (i.kind === 'message' && i.showStatus ? [i.id] : []));
    expect(status).toEqual(['s1', 's3']);
  });

  it('shows no status for a send from before tracking', () => {
    const items = threadLayout([at('old', 5, { is_sent: true, delivery: null })], NOW);
    expect(items.some((i) => i.kind === 'message' && i.showStatus)).toBe(false);
  });

  it('words the live line and leaves it out while offline', () => {
    expect(liveLine({ title: 't', category: 'Just Chatting', viewers: 1234 })).toBe('Live · Just Chatting · 1.2K watching');
    expect(liveLine({ title: 't', category: '', viewers: 12 })).toBe('Live · 12 watching');
    expect(liveLine(null)).toBeNull();
  });

  it('words the sync line', () => {
    expect(syncLine(true, null, NOW)).toBe('Syncing with Twitch...');
    expect(syncLine(false, NOW - 10_000, NOW)).toBe('Synced with Twitch just now');
    expect(syncLine(false, NOW - 5 * MIN, NOW)).toBe('Synced with Twitch 5 min ago');
    expect(syncLine(false, null, NOW)).toBe('Synced with Twitch');
  });

  it('filters the list by name or login', () => {
    const rows = [row({ key: 'a', user_name: 'Marisol', user_login: 'marisol_vg' }), row({ key: 'b', user_name: 'Kestrel', user_login: 'kestrel_tv' })];
    expect(filterRows(rows, 'mari').map((r) => r.key)).toEqual(['a']);
    expect(filterRows(rows, 'KESTREL_').map((r) => r.key)).toEqual(['b']);
    expect(filterRows(rows, '  ')).toHaveLength(2);
  });
});
