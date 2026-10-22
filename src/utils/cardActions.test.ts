import { describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import {
  commandFor,
  HISTORY_PREVIEW,
  loadCardTools,
  reasonFor,
  REASON_MAX,
  requestFor,
  setLowTrust,
  shownHistory,
  undoFor,
  type CardButton,
  type ModHistory,
} from './cardActions';

const button = (over: Partial<CardButton>): CardButton => ({
  id: 't600',
  label: '10m',
  kind: 'timeout',
  action: { kind: 'timeout', seconds: 600 },
  reason: null,
  asks_reason: false,
  danger: false,
  spoken: 'Time out for 10 minutes',
  ...over,
});

const target = { broadcasterId: '1', userId: '9', name: 'Gus' };

describe('cardActions', () => {
  it('passes the action through and sends a reason only when there is one', () => {
    const t = button({});
    expect(requestFor(t, target)).toEqual({
      provider: 'twitch',
      broadcasterId: '1',
      targetUserId: '9',
      targetName: 'Gus',
      action: { kind: 'timeout', seconds: 600 },
      onePress: true,
    });
    const saved = button({ reason: ' spoilers ' });
    expect(requestFor(saved, target).reason).toBe('spoilers');
    expect(requestFor(saved, target, 'typed one').reason).toBe('typed one');
  });

  it('a warning carries the typed reason, cut to Twitch’s limit', () => {
    const warn = button({ id: 'warn', label: 'Warn', kind: 'warn', action: { kind: 'warn' }, asks_reason: true });
    expect(requestFor(warn, target, '  be nice  ')).toMatchObject({ action: { kind: 'warn' }, reason: 'be nice' });
    expect(reasonFor(warn, 'x'.repeat(600))).toHaveLength(REASON_MAX);
    expect(reasonFor(warn, '   ')).toBeNull();
  });

  it('undo lifts a timeout or a ban', () => {
    expect(undoFor(button({})).action).toEqual({ kind: 'unban', timeout: true });
    expect(undoFor(button({ kind: 'ban', action: { kind: 'ban' } })).action).toEqual({ kind: 'unban', timeout: false });
  });

  it('prefills the matching chat command', () => {
    expect(commandFor(button({}), 'gus')).toBe('/timeout gus 600');
    expect(commandFor(button({ reason: 'spam' }), 'gus')).toBe('/timeout gus 600 spam');
    expect(commandFor(button({ kind: 'ban', action: { kind: 'ban' } }), 'gus', 'rude')).toBe('/ban gus rude');
    expect(commandFor(button({ kind: 'unban', action: { kind: 'unban', timeout: true } }), 'gus')).toBe('/untimeout gus');
    expect(commandFor(button({ kind: 'unban', action: { kind: 'unban', timeout: false } }), 'gus')).toBe('/unban gus');
    expect(commandFor(button({ kind: 'warn', action: { kind: 'warn' } }), 'gus')).toBe('/warn gus ');
  });

  it('shows the first few history entries until opened', () => {
    const entry = (i: number) => ({ id: String(i), kind: 'timeout' as const, title: 't', by: null, at_ms: i, when: '', quote: null });
    const history: ModHistory = { status: 'available', entries: [1, 2, 3, 4, 5].map(entry), current: null, note: null };
    expect(shownHistory(history, false)).toHaveLength(HISTORY_PREVIEW);
    expect(shownHistory(history, true)).toHaveLength(5);
    expect(shownHistory(null, true)).toEqual([]);
  });

  it('asks Rust with the history flag and sends low-trust treatments as given', async () => {
    invoke.mockResolvedValue(null);
    await loadCardTools({ channel: 'chan', channelId: '1', userId: '9', history: false, viewerModerates: true });
    expect(invoke).toHaveBeenLastCalledWith('get_chatter_card_tools', {
      provider: 'twitch',
      channel: 'chan',
      channelId: '1',
      userId: '9',
      history: false,
      viewerModerates: true,
    });
    await loadCardTools({ channel: 'chan', channelId: '1', userId: '9' });
    expect(invoke.mock.lastCall?.[1]).toMatchObject({ history: true });
    await setLowTrust({ id: 'restrict', label: 'Restrict', status: 'RESTRICTED', spoken: '', done: '' }, '1', '9', 'chan');
    expect(invoke).toHaveBeenLastCalledWith('update_suspicious_user_status', {
      broadcasterId: '1',
      targetUserId: '9',
      status: 'RESTRICTED',
      channel: 'chan',
    });
  });
});
