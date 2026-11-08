import { describe, expect, it } from 'vitest';
import type { StoredWhisperMessage, WhisperPagePayload, WhisperRowMeta, WhisperUpdatePayload } from '../../types';
import {
  MAX_WHISPER,
  canSend,
  clockTime,
  composerPlaceholder,
  counterText,
  dayLabel,
  failureReason,
  fromPage,
  scalarLength,
  shownName,
  threadItems,
  updateNeedsReread,
  withOlder,
  withRefresh,
  withUpdate,
} from './whisperModel';

const NOW = new Date(2026, 9, 19, 15, 0, 0).getTime();
const MIN = 60_000;
const ME = '99999999';
const THEM = '1000000000';

function msg(id: string, at: number, mine: boolean, over: Partial<StoredWhisperMessage> = {}): StoredWhisperMessage {
  return {
    id,
    from_user_id: mine ? ME : THEM,
    from_user_login: mine ? 'someme' : 'somefriend',
    from_user_name: mine ? 'SomeMe' : 'SomeFriend',
    to_user_id: mine ? THEM : ME,
    to_user_login: mine ? 'somefriend' : 'someme',
    to_user_name: mine ? 'SomeFriend' : 'SomeMe',
    message: `text ${id}`,
    timestamp: at,
    is_sent: mine,
    parts: [],
    deleted: false,
    edited_ms: null,
    origin: 'twitch',
    delivery: null,
    failure: null,
    cut_to: null,
    ...over,
  };
}

function meta(over: Partial<WhisperRowMeta> = {}): WhisperRowMeta {
  return {
    key: THEM,
    user_id: THEM,
    user_login: 'somefriend',
    user_name: 'SomeFriend',
    profile_image_url: null,
    color: null,
    badges: [],
    last_message_timestamp: NOW,
    last_text: '',
    last_parts: [],
    last_is_sent: false,
    unread_count: 0,
    archived: false,
    muted: false,
    ...over,
  };
}

function page(messages: StoredWhisperMessage[], over: Partial<WhisperPagePayload> = {}): WhisperPagePayload {
  return { meta: meta(), messages, cursor: messages[0] ? `c-${messages[0].id}` : null, has_more_local: false, older_on_twitch: false, ...over };
}

function update(message: StoredWhisperMessage | null, over: Partial<WhisperUpdatePayload> = {}): WhisperUpdatePayload {
  return { owner_id: ME, key: THEM, replaced_key: null, meta: meta(), message, removed_message_id: null, loading: null, error: null, ...over };
}

describe('whisper words', () => {
  it('names the person and the composer', () => {
    expect(shownName(meta())).toBe('SomeFriend');
    expect(shownName(meta({ user_name: '' }))).toBe('somefriend');
    expect(shownName(meta({ user_name: '', user_login: '' }))).toBe('Someone');
    expect(composerPlaceholder(meta())).toBe('Whisper SomeFriend');
  });

  it('counts characters as Rust does and stops at the limit', () => {
    expect(scalarLength('a😀b')).toBe(3);
    expect(counterText(449)).toBeNull();
    expect(counterText(450)).toBe('450/500');
    expect(canSend('   ')).toBe(false);
    expect(canSend('😀'.repeat(MAX_WHISPER))).toBe(true);
    expect(canSend('a'.repeat(MAX_WHISPER + 1))).toBe(false);
  });

  it('heads days and gives the reason Rust gave', () => {
    expect(dayLabel(NOW - 60 * MIN, NOW)).toBe('Today');
    expect(dayLabel(NOW - 24 * 60 * MIN, NOW)).toBe('Yesterday');
    expect(failureReason('Cannot send: User not found.')).toBe('Cannot send: User not found.');
    expect(failureReason(new Error('boom'))).toBe('boom');
    expect(failureReason(undefined)).toBe('Failed to send message. Please try again.');
  });
});

describe('the conversation layout', () => {
  it('heads each day and times the end of each run', () => {
    const items = threadItems(
      [msg('a', NOW - 26 * 60 * MIN, false), msg('b', NOW - 30 * MIN, false), msg('c', NOW - 29 * MIN, false), msg('d', NOW - 28 * MIN, true), msg('e', NOW - 2 * MIN, true)],
      NOW,
    );
    expect(items.map((i) => (i.kind === 'day' ? i.label.split(' · ')[0] : i.id))).toEqual(['Yesterday', 'a', 'Today', 'b', 'c', 'd', 'e']);
    const todayHead = items.find((i) => i.kind === 'day' && i.label.startsWith('Today'));
    expect(todayHead && 'label' in todayHead ? todayHead.label : null).toBe(`Today · ${clockTime(NOW - 30 * MIN)}`);
    const item = (id: string) => items.find((i) => i.id === id && i.kind === 'message');
    expect(item('b')).toMatchObject({ last: false, time: null });
    expect(item('c')).toMatchObject({ last: true, mine: false });
    expect(item('d')).toMatchObject({ last: true, mine: true });
  });

  it('says where each send stands, and Sent or Delivered only on the newest', () => {
    const items = threadItems(
      [
        msg('old', NOW - 10 * MIN, true, { delivery: 'delivered' }),
        msg('going', NOW - 9 * MIN, true, { delivery: 'sending' }),
        msg('check', NOW - 8 * MIN, true, { delivery: 'unknown' }),
        msg('bad', NOW - 7 * MIN, true, { delivery: 'not_delivered', failure: 'Cannot send: User not found.' }),
        msg('new', NOW - 6 * MIN, true, { delivery: 'delivered' }),
      ],
      NOW,
    );
    const status = (id: string) => {
      const it = items.find((i) => i.id === id);
      return it && it.kind === 'message' ? it.status : undefined;
    };
    expect(status('old')).toBeNull();
    expect(status('going')?.label).toBe('Sending...');
    expect(status('check')?.label).toBe('Checking...');
    expect(status('bad')).toMatchObject({ label: 'Not delivered', failure: 'Cannot send: User not found.', actions: true });
    expect(status('new')).toMatchObject({ label: 'Delivered', actions: false });
  });

  it('reads a state this build does not know as Sent, and leaves old sends unmarked', () => {
    const items = threadItems(
      [msg('migrated', NOW - 9 * MIN, true), msg('future', NOW - 8 * MIN, true, { delivery: 'queued_elsewhere' as never })],
      NOW,
    );
    const at = (id: string) => items.find((i) => i.id === id);
    expect(at('migrated')).toMatchObject({ status: null });
    expect(at('future')).toMatchObject({ status: { label: 'Sent' } });
  });

  it('says when Twitch cut a whisper', () => {
    const items = threadItems([msg('cut', NOW - MIN, true, { delivery: 'delivered', cut_to: 500 })], NOW);
    expect(items.find((i) => i.id === 'cut')).toMatchObject({ cut: 'Twitch cut this whisper to 500 characters.' });
  });
});

describe('the thread state', () => {
  it('takes an update for its conversation once, and replaces a message by id', () => {
    const start = fromPage(page([msg('a', NOW - 5 * MIN, false)]));
    const sending = msg('l1', NOW - MIN, true, { origin: 'local_sent', delivery: 'sending' });
    const once = withUpdate(start, THEM, update(sending));
    expect(once?.messages.map((m) => m.id)).toEqual(['a', 'l1']);
    expect(withUpdate(once, THEM, update(sending))?.messages).toHaveLength(2);
    const sent = withUpdate(once, THEM, update({ ...sending, delivery: 'sent' }));
    expect(sent?.messages.find((m) => m.id === 'l1')?.delivery).toBe('sent');
    expect(withUpdate(once, '7', update(sending))).toBe(once);
    expect(withUpdate(once, THEM, update(null, { removed_message_id: 'l1' }))?.messages.map((m) => m.id)).toEqual(['a']);
  });

  it('reads again when Twitch\'s copy of a send arrives under a new id', () => {
    const local = msg('l1', NOW - MIN, true, { origin: 'local_sent', delivery: 'sent' });
    const s = fromPage(page([local]));
    const twitchCopy = msg('t1', NOW - MIN, true, { delivery: 'delivered' });
    expect(updateNeedsReread(s, THEM, update(twitchCopy))).toBe(true);
    expect(updateNeedsReread(s, THEM, update(local))).toBe(false);
    expect(updateNeedsReread(fromPage(page([msg('x', NOW, true, { delivery: 'delivered' })])), THEM, update(twitchCopy))).toBe(false);
  });

  it('adds older pages above and keeps them when the newest page is read again', () => {
    const newest = page([msg('c', NOW - 3 * MIN, false), msg('d', NOW - 2 * MIN, false)], { has_more_local: true });
    const s = fromPage(newest);
    const older = withOlder(s, page([msg('a', NOW - 9 * MIN, false), msg('b', NOW - 8 * MIN, false)], { cursor: 'c-a' }));
    expect(older.messages.map((m) => m.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(older.cursor).toBe('c-a');
    expect(older.hasMoreLocal).toBe(false);
    const again = withRefresh(older, page([msg('c', NOW - 3 * MIN, false), msg('d', NOW - 2 * MIN, false), msg('e', NOW - MIN, false)]));
    expect(again.messages.map((m) => m.id)).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(again.cursor).toBe('c-a');
  });
});
