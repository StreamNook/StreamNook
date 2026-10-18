// Run with: npm test
//
// Slash commands run in Rust (services/slash_dispatch.rs, whose own tests
// cover every command). These pin the page side: what is asked of Rust, and
// how its answer is applied in the room the line was typed in. The command
// table checks stay here because the desktop autocomplete still reads it.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn(async (_cmd: string, _args?: Record<string, unknown>): Promise<unknown> => null);
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args) }));

const addToast = vi.fn();
const createClip = vi.fn(async () => undefined);
const appState = {
  addToast,
  createClip,
  currentUser: { user_id: '1', login: 'me', username: 'me', display_name: 'Me' },
  currentStream: null,
  settings: {},
};
vi.mock('../stores/AppStore', () => ({ useAppStore: Object.assign(vi.fn(), { getState: () => appState }) }));
const requestRefresh = vi.fn();
vi.mock('../stores/pinStore', () => ({ usePinStore: { getState: () => ({ requestRefresh }) } }));

const injectSystemMessage = vi.fn();
const sendChannelMessage = vi.fn(async (..._a: unknown[]) => ({ is_sent: true, message_id: 'x', drop_reason: null }));
const hardRefreshChannel = vi.fn(async () => undefined);
vi.mock('../stores/chatConnectionStore', () => ({
  injectSystemMessage: (...a: unknown[]) => injectSystemMessage(...a),
  sendChannelMessage: (...a: unknown[]) => sendChannelMessage(...a),
  hardRefreshChannel: (...a: unknown[]) => hardRefreshChannel(...(a as [])),
  systemSourceFor: () => undefined,
}));
vi.mock('./logger', () => ({ Logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() } }));
vi.mock('./platform', () => ({ IS_MOBILE: false, IS_ANDROID: false }));

const dispatched: string[] = [];
(globalThis as unknown as { window: EventTarget }).window = new EventTarget();
for (const name of ['streamnook-clear-local-chat', 'streamnook-open-user-card']) {
  window.addEventListener(name, () => dispatched.push(name));
}

import { handleSlashCommand, type SlashOutcome } from './commandHandler';
import { COMMAND_ALIASES, COMMAND_DEFINITIONS, RESERVED_TRIGGERS, rankCommandMatches, resolveCommandAlias } from './chatCommands';

const outcome = (patch: Partial<SlashOutcome>): SlashOutcome => ({
  handled: true,
  send: null,
  clear_draft: true,
  toasts: [],
  notices: [],
  ui: null,
  ...patch,
});
const answer = (o: SlashOutcome) => invoke.mockImplementation(async (cmd: string) => (cmd === 'run_slash_command' ? o : null));
const send = vi.fn();
const run = (line: string) => handleSlashCommand(line, '42', 'chan', send);

beforeEach(() => {
  invoke.mockReset();
  addToast.mockReset();
  injectSystemMessage.mockReset();
  sendChannelMessage.mockClear();
  hardRefreshChannel.mockClear();
  send.mockReset();
  dispatched.length = 0;
});

describe('the hand-off to Rust', () => {
  it('passes the whole line with the room, its id and the platform', async () => {
    answer(outcome({}));
    await handleSlashCommand('/SPAM 20  two  spaces --delay 200', '123', 'somechan', send);
    expect(invoke).toHaveBeenCalledWith('run_slash_command', {
      channel: 'somechan',
      broadcasterId: '123',
      text: '/SPAM 20  two  spaces --delay 200',
      depth: 0,
      platform: 'desktop',
    });
  });

  it('leaves a line Rust does not run to the caller', async () => {
    answer(outcome({ handled: false, clear_draft: false }));
    expect(await run('/me waves')).toBe(false);
    expect(send).not.toHaveBeenCalled();
  });

  it('shows a failed call as a toast and keeps it from chat', async () => {
    invoke.mockImplementation(async () => {
      throw 'boom';
    });
    expect(await run('/ban bob')).toBe(true);
    expect(addToast).toHaveBeenCalledWith('Command failed: boom', 'error');
  });
});

describe('applying the answer', () => {
  it('toasts, prints lines in the room it was typed in, and sends', async () => {
    answer(
      outcome({
        toasts: [{ text: 'Banned bob', kind: 'success' }],
        notices: ['one', 'two'],
        send: 'hello',
      }),
    );
    expect(await run('/x')).toBe(true);
    expect(addToast).toHaveBeenCalledWith('Banned bob', 'success');
    expect(injectSystemMessage.mock.calls.map((c) => [c[0], c[1]])).toEqual([
      ['chan', 'one'],
      ['chan', 'two'],
    ]);
    expect(send).toHaveBeenCalledWith('hello');
  });

  it('clears the chat before printing into it', async () => {
    const order: string[] = [];
    window.addEventListener('streamnook-clear-local-chat', () => order.push('clear'), { once: true });
    injectSystemMessage.mockImplementation(() => order.push('notice'));
    answer(outcome({ ui: { kind: 'clear_local_chat' }, notices: ['Cleared messages in this chat (visual only).'] }));
    await run('/clearmessages');
    expect(order).toEqual(['clear', 'notice']);
  });

  it('opens the user card the page listens for', async () => {
    answer(outcome({ ui: { kind: 'open_user_card', user_id: '7', login: 'bob', display_name: 'Bob' } }));
    await run('/user bob');
    expect(dispatched).toContain('streamnook-open-user-card');
  });

  it('reloads this chat for /reconnect, then says so', async () => {
    const order: string[] = [];
    hardRefreshChannel.mockImplementation(async () => {
      order.push('reload');
    });
    injectSystemMessage.mockImplementation(() => order.push('notice'));
    answer(outcome({ ui: { kind: 'reload_chat' }, notices: ['Chat reconnected.'], toasts: [{ text: 'Reconnecting chat...', kind: 'info' }] }));
    await run('/reconnect');
    expect(hardRefreshChannel).toHaveBeenCalledWith('chan', '42');
    expect(order).toEqual(['reload', 'notice']);
  });

  it('starts a clip for /clip', async () => {
    answer(outcome({ ui: { kind: 'create_clip', channel: 'chan', channel_id: '42', title: '' } }));
    await run('/clip');
    expect(createClip).toHaveBeenCalled();
  });

  it('sends a reply through the chat store with its parent', async () => {
    answer(outcome({ send: 'good point', reply_to: 'm-5' }));
    await run('/reply @bob good point');
    expect(sendChannelMessage).toHaveBeenCalledWith('chan', 'good point', expect.objectContaining({ userId: '1' }), 'm-5', null, undefined);
    expect(send).not.toHaveBeenCalled();
  });

  it('sends and pins for /pin <message>', async () => {
    sendChannelMessage.mockResolvedValueOnce({ is_sent: true, message_id: 'x', drop_reason: null });
    answer(outcome({ send: 'giveaway at noon', pin_after_send: true }));
    await run('/pin giveaway at noon');
    expect(sendChannelMessage).toHaveBeenCalledWith(
      'chan',
      'giveaway at noon',
      expect.objectContaining({ userId: '1' }),
      undefined,
      null,
      { pinAfterSend: true },
    );
    expect(addToast).toHaveBeenCalledWith('Sent and pinned', 'success');
    expect(requestRefresh).toHaveBeenCalled();
  });
});

describe('the command table the autocomplete reads', () => {
  it('maps aliases onto one command', () => {
    expect(resolveCommandAlias('b')).toBe('ban');
    expect(resolveCommandAlias('p')).toBe('purge');
    expect(resolveCommandAlias('deletepoll')).toBe('cancelpoll');
    expect(resolveCommandAlias('ban')).toBe('ban');
  });

  it('lists every alias as a built-in name, so the settings page can flag a custom command sharing one', () => {
    for (const alias of COMMAND_ALIASES.keys()) expect(RESERVED_TRIGGERS.has(alias)).toBe(true);
    for (const name of ['spam', 'pyramid']) {
      const def = COMMAND_DEFINITIONS.find((c) => c.name === name);
      expect(def?.category).toBe('Everyone');
      expect(RESERVED_TRIGGERS.has(name)).toBe(true);
    }
  });

  // Enter picks the first suggestion, so the command a typed name runs is first.
  it('puts the exact name first', () => {
    const custom = (name: string) => ({ name, description: '', category: 'Custom' as const, usage: `/${name}` });
    const list = [...COMMAND_DEFINITIONS, custom('p'), custom('ban')];
    expect(rankCommandMatches(list, 'p')[0].name).toBe('p');
    expect(rankCommandMatches(list, 'P')[0].name).toBe('p');
    expect(rankCommandMatches(COMMAND_DEFINITIONS, 'b')[0].name).toBe('ban');
    expect(rankCommandMatches(COMMAND_DEFINITIONS, 'u')[0].name).toBe('unban');
    const ban = rankCommandMatches(list, 'ban').map((c) => `${c.category}:${c.name}`);
    expect(ban.slice(0, 2)).toEqual(['Moderator:ban', 'Custom:ban']);
    expect(rankCommandMatches(COMMAND_DEFINITIONS, 'pol').map((c) => c.name)).toEqual(['poll']);
  });
});
