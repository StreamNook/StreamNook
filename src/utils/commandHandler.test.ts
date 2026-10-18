import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ── environment: just enough of the app for the dispatcher to run ──────────

const invoke = vi.fn(async (_cmd: string, _args?: Record<string, unknown>): Promise<unknown> => null);
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args) }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

const addToast = vi.fn();
const appState = {
  addToast,
  currentUser: { user_id: '1', login: 'me', username: 'me', display_name: 'Me' },
  currentStream: null,
  settings: { chat_commands: { user_commands: [] }, show_mod_logs: false },
  updateSettings: vi.fn(async () => undefined),
  isFavoriteStreamer: vi.fn(() => false),
  toggleFavoriteStreamer: vi.fn(async () => undefined),
  createClip: vi.fn(async () => undefined),
  reloadStreamAndChat: vi.fn(async () => undefined),
};
vi.mock('../stores/AppStore', () => ({ useAppStore: Object.assign(vi.fn(), { getState: () => appState }) }));
vi.mock('../stores/pinStore', () => ({ usePinStore: { getState: () => ({ requestRefresh: vi.fn() }) } }));

const injectSystemMessage = vi.fn();
const sendChannelMessage = vi.fn(async (..._a: unknown[]) => ({ is_sent: true, message_id: 'x', drop_reason: null }));
vi.mock('../stores/chatConnectionStore', () => ({
  getChannelEmotes: vi.fn(() => null),
  hardRefreshChannel: vi.fn(async () => undefined),
  injectSystemMessage: (...a: unknown[]) => injectSystemMessage(...a),
  sendChannelMessage: (...a: unknown[]) => sendChannelMessage(...a),
  systemSourceFor: () => undefined,
  refreshChannelEmotes: vi.fn(async () => null),
  useChatConnectionStore: { getState: () => ({ channels: new Map([['chan', { isConnected: true }]]) }) },
}));
vi.mock('../stores/chatDockStore', () => ({
  dockChat: vi.fn(async () => undefined),
  undockChat: vi.fn(async () => undefined),
  setDockPanelOpen: vi.fn(),
  useChatDockStore: { getState: () => ({ chats: [] }) },
}));
vi.mock('./userChatOverrides', () => ({ setUserColor: vi.fn(), setUserNickname: vi.fn() }));
vi.mock('./nukeEngine', () => ({
  isUserModeratorOf: () => true,
  parseNukeArgs: () => ({ error: 'bad' }),
  executeNuke: vi.fn(),
  executeUndo: vi.fn(),
  stopActiveNukes: () => 0,
}));
vi.mock('./reminderEngine', () => ({
  parseRemindCommand: () => ({ help: true }),
  fireReminderNow: vi.fn(),
  formatReminderList: () => [],
  remindHelpLines: () => ['help'],
}));
vi.mock('./songId', () => ({ recognizeNowPlaying: vi.fn(async () => null), announceSong: vi.fn() }));
vi.mock('./lastWhisper', () => ({ ensureLastWhisperListener: vi.fn(), getLastWhisperFrom: () => null }));
vi.mock('./logger', () => ({ Logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() } }));
vi.mock('./platform', () => ({ IS_MOBILE: false }));
vi.mock('./pinCommand', () => ({ resolvePinTarget: () => ({ error: 'nothing to pin' }) }));

(globalThis as unknown as { window: EventTarget }).window = new EventTarget();

import { handleSlashCommand } from './commandHandler';
import { COMMAND_ALIASES, COMMAND_DEFINITIONS, RESERVED_TRIGGERS, findCommandDefinition, resolveCommandAlias } from './chatCommands';
import { EXTENDED_COMMANDS, helpLines, parseDelay, parseSettingValue, sendPaced } from './extendedCommands';

const run = (line: string) => handleSlashCommand(line, '42', 'chan', vi.fn());
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);
// Bans and timeouts go through Rust `moderate_chatter`; read them back as
// (channel, chatter, seconds or null for a ban, reason).
const bans = () =>
  calls('moderate_chatter')
    .map((c) => c?.request as { broadcasterId?: string; targetUserId?: string; action: { kind: string; seconds?: number }; reason?: string | null })
    .filter((r) => r.action.kind === 'ban' || r.action.kind === 'timeout')
    .map((r) => ({
      broadcasterId: r.broadcasterId,
      targetUserId: r.targetUserId,
      duration: r.action.kind === 'timeout' ? r.action.seconds : null,
      reason: r.reason ?? null,
    }));

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_user_by_login') return { id: '7', login: 'bob', display_name: 'Bob' };
    if (cmd === 'moderate_chatter') return { appliedSeconds: null, message: 'done', undoable: true };
    if (cmd === 'get_active_poll') return { id: 'poll-9' };
    if (cmd.startsWith('slash_')) return 'ok';
    return null;
  });
  addToast.mockReset();
  injectSystemMessage.mockReset();
  sendChannelMessage.mockClear();
});

describe('aliases', () => {
  it('map habits from other clients onto one implementation', () => {
    expect(resolveCommandAlias('b')).toBe('ban');
    expect(resolveCommandAlias('u')).toBe('unban');
    expect(resolveCommandAlias('p')).toBe('purge');
    expect(resolveCommandAlias('sub')).toBe('subscribers');
    expect(resolveCommandAlias('suboff')).toBe('subscribersoff');
    expect(resolveCommandAlias('r9kbeta')).toBe('uniquechat');
    expect(resolveCommandAlias('deletepoll')).toBe('cancelpoll');
    expect(resolveCommandAlias('rejoin')).toBe('reconnect');
    expect(resolveCommandAlias('unbanall')).toBe('massunban');
    expect(resolveCommandAlias('ban')).toBe('ban');
    expect(findCommandDefinition('B')?.name).toBe('ban');
  });

  it('are listed as built-in names, so the settings page can flag a custom command sharing one', () => {
    for (const alias of COMMAND_ALIASES.keys()) expect(RESERVED_TRIGGERS.has(alias)).toBe(true);
  });

  it('never collide with a command name or another alias', () => {
    const names = new Set(COMMAND_DEFINITIONS.map((c) => c.name));
    const seen = new Set<string>();
    for (const c of COMMAND_DEFINITIONS) {
      for (const a of c.aliases ?? []) {
        expect(names.has(a)).toBe(false);
        expect(seen.has(a)).toBe(false);
        seen.add(a);
      }
    }
    expect(names.size).toBe(COMMAND_DEFINITIONS.length);
  });
});

describe('every listed command does something', () => {
  // The switch's own case labels, read from the source so a new listing
  // without a handler fails here instead of shipping dead.
  const source = readFileSync(join(__dirname, 'commandHandler.ts'), 'utf8');
  const switchCases = new Set(Array.from(source.matchAll(/case '([\w-]+)':/g), (m) => m[1]));

  it('has a real handler for each listed name (only /me goes to chat as text)', () => {
    const missing = COMMAND_DEFINITIONS.filter((c) => !c.hint)
      .map((c) => c.name)
      .filter((n) => !switchCases.has(n) && !EXTENDED_COMMANDS.has(n));
    expect(missing).toEqual([]);
  });

  it('handles each name and alias, bare and with its examples, without IRC passthrough', async () => {
    const lines: string[] = [];
    for (const c of COMMAND_DEFINITIONS) {
      if (c.hint || c.name === 'me' || c.name === 'spam' || c.name === 'pyramid') continue;
      lines.push(`/${c.name}`, ...(c.aliases ?? []).map((a) => `/${a}`), ...(c.examples ?? []));
    }
    for (const line of lines) {
      if (/^\/(timer|chain|foreach|runin)\b/.test(line)) continue; // scripted below
      const handled = await run(line);
      expect({ line, handled }).toEqual({ line, handled: true });
    }
    expect(calls('send_chat_message')).toEqual([]);
  });

  it('leaves /me to chat', async () => {
    expect(await run('/me waves')).toBe(false);
  });
});

describe('dispatch', () => {
  it('runs aliases through the same call as the full name', async () => {
    await run('/b bob spamming');
    await run('/ban bob spamming');
    const made = bans();
    expect(made).toHaveLength(2);
    expect(made[0]).toEqual(made[1]);
    expect(made[0]).toMatchObject({ broadcasterId: '42', targetUserId: '7', duration: null, reason: 'spamming' });
  });

  it('/p and /purge time out for one second', async () => {
    await run('/p bob');
    await run('/purge bob rude');
    expect(bans()).toEqual([
      { broadcasterId: '42', targetUserId: '7', duration: 1, reason: null },
      { broadcasterId: '42', targetUserId: '7', duration: 1, reason: 'rude' },
    ]);
  });

  it('/sub turns on sub-only chat', async () => {
    await run('/sub');
    expect(calls('update_chat_settings')).toEqual([{ broadcasterId: '42', settings: { subscriber_mode: true } }]);
  });

  it('/deletepoll archives the running poll', async () => {
    await run('/deletepoll');
    expect(calls('end_poll')).toEqual([{ broadcasterId: '42', pollId: 'poll-9', status: 'ARCHIVED' }]);
  });

  it('/vote passes the choice to Rust', async () => {
    await run('/vote 2');
    expect(calls('slash_vote')).toEqual([{ channelId: '42', choice: '2' }]);
  });

  it('/followage reads user then channel', async () => {
    await run('/followage alice otherchan');
    await run('/followed');
    expect(calls('slash_followage')).toEqual([
      { channel: 'otherchan', user: 'alice' },
      { channel: 'chan', user: null },
    ]);
  });

  it('/gift opens the sub page instead of IRC', async () => {
    await run('/gift 5');
    expect(calls('open_subscribe_window')).toEqual([{ channelLogin: 'chan', title: 'Gift subs to chan', provider: 'twitch' }]);
  });

  it('/automod_approve is /automod allow', async () => {
    await run('/automod_approve abc');
    await run('/automod deny');
    expect(calls('slash_automod')).toEqual([
      { channel: 'chan', messageId: 'abc', allow: true },
      { channel: 'chan', messageId: null, allow: false },
    ]);
  });

  it('/reply finds their last message and replies to it', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'slash_last_message' ? { id: 'm-5', display_name: 'Bob' } : null));
    await run('/reply @bob good point');
    expect(sendChannelMessage).toHaveBeenCalledWith('chan', 'good point', expect.objectContaining({ userId: '1' }), 'm-5');
  });

  it('missing arguments show usage instead of reaching chat', async () => {
    expect(await run('/ban')).toBe(true);
    expect(addToast).toHaveBeenCalledWith('Usage: /ban <username> [reason]', 'info');
    expect(await run('/purge')).toBe(true);
    expect(addToast).toHaveBeenCalledWith('Usage: /purge <username> [reason]', 'info');
    expect(bans()).toEqual([]);
  });

  it('/editor asks for confirmation before granting', async () => {
    appState.currentUser = { ...appState.currentUser, login: 'chan' };
    try {
      await run('/editor bob');
      expect(calls('slash_editor')).toEqual([]);
      await run('/editor bob');
      expect(calls('slash_editor')).toEqual([{ broadcasterId: '42', target: 'bob', add: true }]);
    } finally {
      appState.currentUser = { ...appState.currentUser, login: 'me' };
    }
  });

  it('broadcaster-only commands refuse viewers before calling Twitch', async () => {
    await run('/raidsend');
    await run('/massunban');
    expect(calls('slash_raid_now')).toEqual([]);
    expect(calls('slash_mass_unban_start')).toEqual([]);
  });

  it('/chain runs each step in order', async () => {
    await run('/chain /slow 10 | /slowoff');
    expect(calls('update_chat_settings')).toEqual([
      { broadcasterId: '42', settings: { slow_mode: true, slow_mode_wait_time: 10 } },
      { broadcasterId: '42', settings: { slow_mode: false } },
    ]);
  });

  it('/foreach fills $ for each item', async () => {
    await run('/foreach alice,bob /shoutout $');
    expect(calls('send_shoutout')).toHaveLength(2);
  });

  it('/say sends text without running it as a command', async () => {
    const send = vi.fn();
    await handleSlashCommand('/say !discord', '42', 'chan', send);
    expect(send).toHaveBeenCalledWith('!discord');
  });
});

describe('helpers', () => {
  it('/help lists a command from its definition', () => {
    const lines = helpLines('b');
    expect(lines[0]).toContain('/ban <username> [reason]');
    expect(lines.join(' ')).toContain('/b');
    expect(helpLines('nope')[0]).toContain('no /nope');
    expect(helpLines(undefined).length).toBeGreaterThan(2);
  });

  it('parses delays', () => {
    expect(parseDelay('30s')).toBe(30_000);
    expect(parseDelay('1h30m')).toBe(5_400_000);
    expect(parseDelay('45')).toBe(45_000);
    expect(parseDelay('soon')).toBeNull();
  });

  it('reads setting values as JSON first', () => {
    expect(parseSettingValue('15')).toBe(15);
    expect(parseSettingValue('true')).toBe(true);
    expect(parseSettingValue('hello there')).toBe('hello there');
  });

  it('sends scripted lines from Rust, paced by the limiter, and reports a refusal', async () => {
    invoke.mockImplementation(async () => ({ is_sent: true, drop_reason: null }));
    expect(await sendPaced('chan', 'hi', '42')).toBe(true);
    expect(invoke.mock.calls[0]).toEqual(['chat_send_paced', { channel: 'chan', channelId: '42', text: 'hi' }]);
    expect(sendChannelMessage).not.toHaveBeenCalled();
    invoke.mockImplementation(async () => {
      throw 'Chat is in sub-only mode';
    });
    expect(await sendPaced('chan', 'again')).toBe(false);
    expect(injectSystemMessage).toHaveBeenCalledWith('chan', 'Not sent to chan: Chat is in sub-only mode', undefined, undefined);
  });
});
