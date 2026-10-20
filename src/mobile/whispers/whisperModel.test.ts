import { describe, expect, it } from 'vitest';
import type { Whisper, WhisperListRow, WhisperUpdate } from '../../types';
import {
  applyToThread,
  composerPlaceholder,
  dayLabel,
  failureReason,
  previewLine,
  rowTime,
  shownName,
  threadItems,
  type PendingWhisper,
} from './whisperModel';

const NOW = new Date(2026, 9, 19, 15, 0, 0).getTime();
const MIN = 60_000;

function row(over: Partial<WhisperListRow> = {}): WhisperListRow {
  return {
    key: '42',
    user_id: '42',
    user_login: 'bob',
    user_name: 'Bob',
    profile_image_url: null,
    last_message_timestamp: NOW - 5 * MIN,
    unread_count: 0,
    last_text: 'see you there',
    last_is_sent: false,
    ...over,
  };
}

function msg(id: string, at: number, mine: boolean, text = 'hi'): Whisper {
  return {
    id,
    from_user_id: mine ? '1' : '42',
    from_user_login: mine ? 'me' : 'bob',
    from_user_name: mine ? 'Me' : 'Bob',
    to_user_id: mine ? '42' : '1',
    to_user_login: mine ? 'bob' : 'me',
    to_user_name: mine ? 'Bob' : 'Me',
    message: text,
    timestamp: at,
    is_sent: mine,
  };
}

describe('whisper words', () => {
  it('names the person and marks your own last line', () => {
    expect(shownName(row())).toBe('Bob');
    expect(shownName(row({ user_name: '' }))).toBe('bob');
    expect(shownName(row({ user_name: '', user_login: '' }))).toBe('Someone');
    expect(previewLine(row())).toBe('see you there');
    expect(previewLine(row({ last_is_sent: true }))).toBe('You: see you there');
    expect(previewLine(row({ last_text: '' }))).toBe('');
    expect(composerPlaceholder(row())).toBe('Whisper Bob');
  });

  it('dates rows and days the way Mentions does', () => {
    expect(rowTime(NOW - 10_000, NOW)).toBe('now');
    expect(rowTime(NOW - 5 * MIN, NOW)).toBe('5m');
    expect(rowTime(NOW - 3 * 60 * MIN, NOW)).toBe('3h');
    expect(rowTime(NOW - 2 * 24 * 60 * MIN, NOW)).toBe('2d');
    expect(dayLabel(NOW - 60 * MIN, NOW)).toBe('Today');
    expect(dayLabel(NOW - 24 * 60 * MIN, NOW)).toBe('Yesterday');
  });

  it('shows the reason Rust gave, else the plain fallback', () => {
    expect(failureReason('Cannot send: User not found.')).toBe('Cannot send: User not found.');
    expect(failureReason(new Error('boom'))).toBe('boom');
    expect(failureReason(undefined)).toBe('Failed to send message. Please try again.');
  });
});

describe('the conversation view', () => {
  it('heads each day and times the end of each run', () => {
    const messages = [
      msg('a', NOW - 26 * 60 * MIN, false),
      msg('b', NOW - 30 * MIN, false),
      msg('c', NOW - 29 * MIN, false),
      msg('d', NOW - 28 * MIN, true),
      msg('e', NOW - 2 * MIN, true),
    ];
    const items = threadItems(messages, [], NOW);
    expect(items.map((i) => (i.kind === 'day' ? i.label : i.id))).toEqual(['Yesterday', 'a', 'Today', 'b', 'c', 'd', 'e']);
    const last = (id: string) => items.find((i) => i.id === id && i.kind === 'message');
    // b and c are one run; d starts yours; e is more than five minutes later.
    expect(last('b')).toMatchObject({ last: false, time: null });
    expect(last('c')).toMatchObject({ last: true, mine: false });
    expect(last('d')).toMatchObject({ last: true, mine: true });
    expect(last('e')).toMatchObject({ last: true });
  });

  it('puts what is still on its way after the thread, under today', () => {
    const pending: PendingWhisper = { localId: 'l1', key: '42', text: 'yo', status: 'sending', atMs: NOW };
    const items = threadItems([msg('a', NOW - 26 * 60 * MIN, false)], [pending], NOW);
    expect(items.map((i) => i.kind)).toEqual(['day', 'message', 'day', 'pending']);
    expect(threadItems([], [pending], NOW).map((i) => i.kind)).toEqual(['pending']);
  });

  it('takes an update for its conversation once', () => {
    const update: WhisperUpdate = {
      owner_id: '1',
      key: '42',
      replaced_key: null,
      meta: {
        user_id: '42',
        user_login: 'bob',
        user_name: 'Bob',
        profile_image_url: 'pic',
        last_message_timestamp: NOW,
        unread_count: 0,
      },
      message: msg('z', NOW, false),
    };
    const once = applyToThread(null, '42', update);
    expect(once?.messages.map((m) => m.id)).toEqual(['z']);
    expect(applyToThread(once, '42', update)?.messages).toHaveLength(1);
    // Another conversation's update changes nothing here.
    expect(applyToThread(once, '7', update)).toBe(once);
    // A conversation moved from its login to its id still takes it.
    expect(applyToThread(null, 'bob', { ...update, replaced_key: 'bob' })?.profile_image_url).toBe('pic');
  });
});
