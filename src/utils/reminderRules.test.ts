// "Make it a rule" and "Turn back on" as the UI calls them, and the shapes they send.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
const refresh = vi.fn(async () => null);
vi.mock('../hooks/useAutomationStatus', () => ({ refreshAutomationStatus: () => refresh() }));

import { automationsApi, blankRule, duplicateRule } from './automations';
import {
  convertReminder,
  dismissWhatChanges,
  restoreReminder,
  ruleMadeFrom,
  useWhatChangesStore,
} from './reminderRules';

beforeEach(() => {
  invoke.mockReset();
  refresh.mockClear();
  useWhatChangesStore.setState({ byRule: {} });
});

describe('the three commands', () => {
  it('invokes them by literal name with the shapes Rust reads', async () => {
    invoke.mockResolvedValue([]);
    await automationsApi.setReminders({ op: 'set_enabled', id: 'r1', enabled: true });
    await automationsApi.setReminders({ op: 'upsert', reminder: { id: 'r1', message: 'hi' } });
    await automationsApi.setReminders({ op: 'remove', id: 'r1' });
    await automationsApi.setReminders({ op: 'move', id: 'r1', to: 2 });
    expect(invoke.mock.calls).toEqual([
      ['automations_set_reminders', { op: { op: 'set_enabled', id: 'r1', enabled: true } }],
      ['automations_set_reminders', { op: { op: 'upsert', reminder: { id: 'r1', message: 'hi' } } }],
      ['automations_set_reminders', { op: { op: 'remove', id: 'r1' } }],
      ['automations_set_reminders', { op: { op: 'move', id: 'r1', to: 2 } }],
    ]);
    invoke.mockClear();
    await automationsApi.convertReminder('r1');
    await automationsApi.restoreReminder('r1');
    expect(invoke.mock.calls).toEqual([
      ['automations_convert_reminder', { id: 'r1' }],
      ['automations_restore_reminder', { id: 'r1' }],
    ]);
  });
});

describe('making a reminder a rule', () => {
  it('keeps "What changes" for the new rule and reads the status again', async () => {
    invoke.mockResolvedValue({
      rule_id: 'r-abc',
      changes: [{ field: 'repeat_count', words: 'A rule posts once each time.' }],
      errors: [],
    });
    const answer = await convertReminder('r1');
    expect(answer.rule_id).toBe('r-abc');
    expect(useWhatChangesStore.getState().byRule['r-abc']).toEqual([
      { field: 'repeat_count', words: 'A rule posts once each time.' },
    ]);
    expect(refresh).toHaveBeenCalledTimes(1);
    dismissWhatChanges('r-abc');
    expect(useWhatChangesStore.getState().byRule['r-abc']).toBeUndefined();
  });

  it('keeps nothing and says why when Rust refuses', async () => {
    invoke.mockRejectedValue('Already a rule');
    await expect(convertReminder('r1')).rejects.toBe('Already a rule');
    expect(useWhatChangesStore.getState().byRule).toEqual({});
    expect(refresh).not.toHaveBeenCalled();
  });

  it('turns back on and reads the status again', async () => {
    invoke.mockResolvedValue(null);
    await restoreReminder('r1');
    expect(invoke).toHaveBeenCalledWith('automations_restore_reminder', { id: 'r1' });
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});

describe('ruleMadeFrom', () => {
  const rule = (id: string, origin: string | null) => ({ id, rule: { origin } }) as never;
  it('finds the rule by its origin and says nothing when it is gone', () => {
    const status = { rules: [rule('r-1', null), rule('r-2', 'reminder:r1'), { id: 'r-3', rule: null } as never] };
    expect(ruleMadeFrom(status, 'r1')).toBe('r-2');
    expect(ruleMadeFrom(status, 'r9')).toBeNull();
    expect(ruleMadeFrom(null, 'r1')).toBeNull();
  });
});

describe('a copy of a converted rule', () => {
  it('starts without what "Make it a rule" stored and keeps "Only in chats you have open"', () => {
    const rule = blankRule({ provider: 'twitch', id: '1', login: 'me', name: 'me' });
    const converted = {
      ...rule,
      claim_as: {
        account_id: '1',
        kind: 'keyword' as const,
        family_hash: '0123456789abcdef',
        seconds: 0,
        cooldown_s: 300,
        trigger_kind: 'message',
        scope_kind: 'own',
        scope_key: '',
        send_to: null,
      },
      only_if: { ...rule.only_if, only_while_open: true, lead_moderator_excluded: true, only_while_watching: true },
    };
    const copy = duplicateRule(converted);
    expect(copy.claim_as).toBeNull();
    expect(copy.only_if).toMatchObject({ only_while_open: true, lead_moderator_excluded: false, only_while_watching: false });
  });
});
