import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import {
  COMMAND_FORMS,
  actionOf,
  blankRule,
  isOutward,
  joinCommand,
  offeredP2Actions,
  pickCategory,
  resolveSetGames,
  saveRuleChecked,
  shownArgument,
  splitCommand,
  triggerOf,
  withTrigger,
} from './automations';
import { nextTimeout, timeoutWords } from '../components/settings/automations/P2Fields';

const owner = { provider: 'twitch', id: '1', login: 'me', name: 'me' };

describe('P2 editor helpers', () => {
  it('the command forms equal the Rust fixture', () => {
    const fixture = JSON.parse(readFileSync('src-tauri/tests/fixtures/automation_command_forms.json', 'utf8'));
    expect(COMMAND_FORMS.map(({ word, label, arg, own_timed_only }) => ({ word, label, arg, own_timed_only }))).toEqual(fixture);
  });

  it('splits and joins a command line', () => {
    expect(splitCommand('/Slow  60 ')).toEqual(['slow', '60']);
    expect(splitCommand('/marker')).toEqual(['marker', '']);
    expect(joinCommand('slow', ' 60 ')).toBe('/slow 60');
    expect(joinCommand('slowoff', '')).toBe('/slowoff');
  });

  it('picks a category the way pick_category does and stores its own name', () => {
    const rows = [
      { id: '1', name: 'Artifact' },
      { id: '509660', name: 'Art' },
    ];
    expect(pickCategory(rows, 'art')).toBe('Art');
    expect(pickCategory(rows, 'arts')).toBeNull();
    expect(pickCategory([{ id: '0', name: 'Art' }], 'Art')).toBeNull();
  });

  describe('/setgame at save', () => {
    beforeEach(() => invoke.mockReset());
    const setgame = (line: string) => ({ ...actionOf('command'), line }) as ReturnType<typeof actionOf>;

    it('stores the category by its own name and leaves other actions alone', async () => {
      invoke.mockResolvedValue([{ id: '509660', name: 'Art' }]);
      const notify = actionOf('notify');
      const { actions, problems } = await resolveSetGames([notify, setgame('/setgame art')]);
      expect(problems).toEqual([]);
      expect(actions[0]).toBe(notify);
      expect(actions[1]).toMatchObject({ kind: 'command', line: '/setgame Art' });
      expect(invoke).toHaveBeenCalledWith('search_categories', { query: 'art', limit: 10 });
    });

    it('says "No category named X" on the action when nothing matches exactly', async () => {
      invoke.mockResolvedValue([{ id: '1', name: 'Artifact' }]);
      const { problems } = await resolveSetGames([setgame('/setgame art ')]);
      expect(problems).toEqual([{ path: 'actions[0].line', message: 'No category named art' }]);
    });

    it('says it could not look the categories up, with the reason, when the lookup fails', async () => {
      invoke.mockRejectedValue('Sign in to Twitch first');
      const { problems } = await resolveSetGames([actionOf('notify'), setgame('/setgame art')]);
      expect(problems).toEqual([{ path: 'actions[1].line', message: "Couldn't look up categories: Sign in to Twitch first" }]);
    });

    it('sends nothing to Rust on a miss or a failed lookup, and stores the category name on a hit', async () => {
      const rule = { ...blankRule(owner), actions: [setgame('/setgame art')] };
      invoke.mockResolvedValueOnce([{ id: '1', name: 'Artifact' }]);
      const missed = await saveRuleChecked(rule, 0);
      expect(missed.answer).toBeNull();
      expect(missed.problems[0].message).toBe('No category named art');
      invoke.mockRejectedValueOnce('Twitch is not answering');
      expect((await saveRuleChecked(rule, 0)).answer).toBeNull();
      expect(invoke.mock.calls.filter(([name]) => name === 'automations_save_rule')).toHaveLength(0);
      invoke.mockResolvedValueOnce([{ id: '509660', name: 'Art' }]);
      invoke.mockResolvedValueOnce({ rule, errors: [], saved: true });
      const hit = await saveRuleChecked(rule, 0);
      expect(hit.answer?.saved).toBe(true);
      expect(hit.rule.actions[0]).toMatchObject({ line: '/setgame Art' });
      const save = invoke.mock.calls.find(([name]) => name === 'automations_save_rule');
      expect(save?.[1]).toMatchObject({ expectedUpdatedMs: 0 });
    });

    it('keeps a multi-word argument while it is typed key by key', () => {
      let typed: { word: string; text: string } | null = null;
      let line = '/setgame';
      for (const text of ['J', 'Ju', 'Jus', 'Just', 'Just ', 'Just C', 'Just Ch', 'Just Chatting']) {
        typed = { word: 'setgame', text };
        line = joinCommand('setgame', text);
        expect(shownArgument(typed, 'setgame', splitCommand(line)[1])).toBe(text);
      }
      expect(line).toBe('/setgame Just Chatting');
      expect(shownArgument(typed, 'settitle', '')).toBe('');
    });

    it('asks nothing for a name with a variable, another command or no name', async () => {
      const { actions, problems } = await resolveSetGames([setgame('/setgame {game}'), setgame('/slow 30'), setgame('/setgame')]);
      expect(problems).toEqual([]);
      expect(actions).toHaveLength(3);
      expect(invoke).not.toHaveBeenCalled();
    });
  });

  it('constructs every new kind with its defaults', () => {
    expect(triggerOf('timer')).toEqual({ kind: 'timer', every_seconds: 900, min_messages: 10 });
    expect(triggerOf('whisper')).toEqual({ kind: 'whisper', text: null });
    expect(actionOf('timeout')).toMatchObject({ kind: 'timeout', seconds: 600 });
    expect(actionOf('announce')).toMatchObject({ color: 'primary' });
    expect(actionOf('wait')).toMatchObject({ seconds: 5 });
  });

  it('isOutward matches Rust', () => {
    for (const k of ['notify', 'log', 'join_chat', 'leave_chat', 'wait'] as const) expect(isOutward(actionOf(k))).toBe(false);
    for (const k of ['reply', 'send', 'command', 'whisper', 'timeout', 'delete', 'pin', 'announce', 'shoutout'] as const) {
      expect(isOutward(actionOf(k))).toBe(true);
    }
  });

  it('moderation is offered on chat messages only, Shoutout also on a raid, and only for the main account', () => {
    const msg = triggerOf('message');
    expect(offeredP2Actions(msg, true)).toEqual(expect.arrayContaining(['timeout', 'delete', 'pin', 'shoutout']));
    expect(offeredP2Actions(triggerOf('timer'), true)).not.toContain('timeout');
    expect(offeredP2Actions(triggerOf('timer'), true)).not.toContain('announce');
    const raid = { ...triggerOf('channel_event'), events: ['raid'] } as ReturnType<typeof triggerOf>;
    expect(offeredP2Actions(raid, true)).toContain('shoutout');
    expect(offeredP2Actions(raid, true)).not.toContain('timeout');
    expect(offeredP2Actions(msg, false)).toEqual(['wait', 'log']);
    expect(offeredP2Actions(triggerOf('whisper'), true)).toContain('whisper');
  });

  it('a timed trigger leaves any-open for your channel and a whisper rule reads own', () => {
    const rule = { ...blankRule(owner), only_if: { ...blankRule(owner).only_if, channels: { kind: 'any_open' } as const } };
    expect(withTrigger(rule, 'timer').only_if.channels.kind).toBe('own');
    expect(withTrigger(rule, 'whisper').only_if.channels.kind).toBe('own');
    expect(withTrigger(rule, 'timer').only_if.only_live).toBe(true);
  });

  it('timeout steps stay inside 1 second to 14 days', () => {
    expect(nextTimeout(600, 1)).toBe(1800);
    expect(nextTimeout(1_209_600, 1)).toBe(1_209_600);
    expect(nextTimeout(1, -1)).toBe(1);
    expect(timeoutWords(86400)).toBe('1 day');
    expect(timeoutWords(600)).toBe('10 min');
  });
});
