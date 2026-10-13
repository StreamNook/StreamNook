// Run with: npm test
//
// The slash command dispatcher around scripts (/chain, /foreach, /timer),
// confirmations, the /spam hand-off, custom-command precedence and argument
// edge cases. Each case pins a bug found before release.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn(async (_cmd: string, _args?: Record<string, unknown>): Promise<unknown> => null);
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args) }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

const addToast = vi.fn();
interface TestUserCommand {
  id: string;
  trigger: string;
  expansion: string;
  enabled: boolean;
  require_slash?: boolean;
}
const appState = {
  addToast,
  currentUser: { user_id: '1', login: 'chan', username: 'chan', display_name: 'Chan' },
  currentStream: null,
  settings: { chat_commands: { user_commands: [] as TestUserCommand[] }, show_mod_logs: false },
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
import { helpLines, parseDelay, runExtendedCommand } from './extendedCommands';
import { COMMAND_DEFINITIONS, rankCommandMatches } from './chatCommands';

// The signed-in account is the broadcaster of `chan` (id 42) in these cases.
const send = vi.fn();
const run = (line: string) => handleSlashCommand(line, '42', 'chan', send);
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);
const sentTexts = () => [
  ...send.mock.calls.map((c) => String(c[0])),
  ...sendChannelMessage.mock.calls.map((c) => String(c[1])),
  ...calls('chat_send_paced').map((c) => String(c?.text)),
];

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_user_by_login') return { id: '7', login: 'bob', display_name: 'Bob' };
    if (cmd === 'run_chat_burst') return { message: 'Spam started: 2 messages at 1000 ms apart. /spam stop ends it', status: null };
    if (cmd === 'chat_send_paced') return { is_sent: true, drop_reason: null };
    if (cmd.startsWith('slash_')) return 'ok';
    return null;
  });
  addToast.mockReset();
  injectSystemMessage.mockReset();
  sendChannelMessage.mockClear();
  send.mockReset();
  appState.settings.chat_commands.user_commands = [];
});

afterEach(() => {
  vi.useRealTimers();
});

describe('scripts and confirmations', () => {
  it('a lone /massunban asks before it starts', async () => {
    await run('/massunban');
    expect(calls('slash_mass_unban_start')).toEqual([]);
  });

  // The "run it again within 20 seconds" confirmation counts only when typed:
  // a custom command, /timer or /foreach must never confirm on its own.
  it('a script cannot confirm /massunban on its own', async () => {
    await run('/chain /massunban | /massunban');
    expect(calls('slash_mass_unban_start')).toEqual([]);
    expect(injectSystemMessage).toHaveBeenCalledWith(
      'chan',
      expect.stringContaining("can't run from /chain, /foreach, /timer or /runin"),
      undefined,
      undefined,
    );
  });

  it('a script cannot confirm /editor on its own', async () => {
    await run('/foreach x,y /editor bob $');
    expect(calls('slash_editor')).toEqual([]);
  });

  it('a typed second /massunban still confirms', async () => {
    await run('/massunban');
    await run('/massunban');
    expect(calls('slash_mass_unban_start')).toEqual([{ broadcasterId: '42' }]);
  });

  // A command that goes to chat as text (/me, or one nothing knows) is sent,
  // not dropped, when it is a step of a script.
  it('/me inside /chain still reaches chat', async () => {
    await run('/chain /me waves | /me dances');
    expect(sentTexts()).toEqual(['/me waves', '/me dances']);
    expect(calls('chat_send_paced')).toEqual([
      { channel: 'chan', channelId: '42', text: '/me waves' },
      { channel: 'chan', channelId: '42', text: '/me dances' },
    ]);
  });

  // A delayed text command goes to the channel the timer was set in, not to
  // whatever channel the chat box shows when it fires.
  it('/timer sends a delayed /say to the channel it was set in', async () => {
    vi.useFakeTimers();
    await run('/timer 5s /say hi');
    await vi.advanceTimersByTimeAsync(5_000);
    expect(send).not.toHaveBeenCalled();
    expect(calls('chat_send_paced')).toEqual([{ channel: 'chan', channelId: '42', text: 'hi' }]);
  });

  it('/timer sends plain text to the channel it was set in', async () => {
    vi.useFakeTimers();
    await run('/timer 5s hello');
    await vi.advanceTimersByTimeAsync(5_000);
    expect(calls('chat_send_paced')).toEqual([{ channel: 'chan', channelId: '42', text: 'hello' }]);
  });

  // A timer whose chat was closed or switched away from still sends (Rust
  // needs no open chat), and a refusal shows where it can be seen: a toast
  // when that chat is not open here.
  it('a refused send to a chat that is not open shows a toast', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_user_by_login') return { id: '9', login: 'other', display_name: 'Other' };
      if (cmd === 'chat_send_paced') throw 'Chat is in emote-only mode, so text messages would not be accepted';
      return null;
    });
    await run('/runin other hello there');
    expect(calls('chat_send_paced')).toEqual([{ channel: 'other', channelId: '9', text: 'hello there' }]);
    expect(addToast).toHaveBeenCalledWith(
      'Not sent to other: Chat is in emote-only mode, so text messages would not be accepted',
      'error',
    );
    expect(injectSystemMessage).not.toHaveBeenCalled();
  });

  it('/reconnect says so after the refresh, not before it', async () => {
    await run('/reconnect');
    expect(addToast).toHaveBeenCalledWith('Reconnecting chat...', 'info');
    expect(injectSystemMessage).toHaveBeenCalledWith('chan', 'Chat reconnected.', undefined, undefined);
  });

  it('a /say typed directly uses the chat box send', async () => {
    await run('/say hi');
    expect(send).toHaveBeenCalledWith('hi');
    expect(sendChannelMessage).not.toHaveBeenCalled();
  });
});

describe('/spam and /pyramid replies', () => {
  // In the chat the command was typed in, however many chats are open.
  it('the reply is printed in the chat the command was typed in', async () => {
    await run('/spam 2 hi');
    expect(injectSystemMessage).toHaveBeenCalledWith('chan', expect.stringContaining('Spam started'), undefined, undefined);
  });
});

describe('arguments', () => {
  it('/ban and /b accept an @name', async () => {
    await run('/b @bob spam');
    expect(calls('get_user_by_login')).toEqual([{ login: 'bob' }]);
    expect(calls('ban_user')).toEqual([{ broadcasterId: '42', targetUserId: '7', duration: null, reason: 'spam' }]);
  });

  it('/timeout and /untimeout accept an @name', async () => {
    await run('/timeout @bob 60');
    await run('/untimeout @bob');
    expect(calls('get_user_by_login')).toEqual([{ login: 'bob' }, { login: 'bob' }]);
  });

  it('/timeout reads a duration with a unit', async () => {
    await run('/timeout bob 10m');
    await run('/timeout bob 1h30m too loud');
    await run('/timeout bob 90');
    expect(calls('ban_user').map((c) => [c?.duration, c?.reason])).toEqual([
      [600, null],
      [5400, 'too loud'],
      [90, null],
    ]);
  });

  it('/timeout without a length keeps the whole reason', async () => {
    await run('/timeout bob calm down');
    expect(calls('ban_user')).toEqual([expect.objectContaining({ duration: 600, reason: 'calm down' })]);
  });

  it('lengths take weeks and days too', () => {
    expect(parseDelay('2d')).toBe(2 * 86_400_000);
    expect(parseDelay('1w1d')).toBe(8 * 86_400_000);
    expect(parseDelay('1h30m')).toBe(5_400_000);
    expect(parseDelay('10x')).toBeNull();
  });

  it('/purge and /logs strip the @', async () => {
    await run('/purge @bob');
    await run('/logs @bob');
    expect(calls('get_user_by_login')).toEqual([{ login: 'bob' }]);
    expect(calls('slash_recent_messages')).toEqual([{ channel: 'chan', login: 'bob', limit: 10 }]);
  });
});

describe('custom commands and built-in names', () => {
  // A built-in name or alias added after people saved custom commands never
  // takes one over: a moderator's saved /p must not start purging users.
  it('a saved custom /p keeps running the saved text', async () => {
    appState.settings.chat_commands.user_commands = [
      { id: 'u1', trigger: 'p', expansion: 'PogChamp {1}', enabled: true, require_slash: true },
    ];
    await run('/p nice');
    expect(calls('ban_user')).toEqual([]);
    expect(send).toHaveBeenCalledWith('PogChamp nice');
  });

  it('a saved custom /lurk wins over the built-in /lurk', async () => {
    appState.settings.chat_commands.user_commands = [
      { id: 'u2', trigger: 'lurk', expansion: 'brb, lurking', enabled: true, require_slash: true },
    ];
    await run('/lurk');
    expect(send).toHaveBeenCalledWith('brb, lurking');
  });

  // Enter picks the first suggestion, so the command a typed name runs is first.
  it('the autocomplete puts the exact name first', () => {
    const custom = (name: string) => ({ name, description: '', category: 'Custom' as const, usage: `/${name}` });
    const list = [...COMMAND_DEFINITIONS, custom('p'), custom('ban')];
    expect(rankCommandMatches(list, 'p')[0].name).toBe('p');
    expect(rankCommandMatches(list, 'P')[0].name).toBe('p');
    // An alias puts the command it runs first.
    expect(rankCommandMatches(COMMAND_DEFINITIONS, 'b')[0].name).toBe('ban');
    expect(rankCommandMatches(COMMAND_DEFINITIONS, 'u')[0].name).toBe('unban');
    // A custom command a built-in always beat comes after that built-in.
    const ban = rankCommandMatches(list, 'ban').map((c) => `${c.category}:${c.name}`);
    expect(ban.slice(0, 2)).toEqual(['Moderator:ban', 'Custom:ban']);
    // A plain prefix keeps list order.
    expect(rankCommandMatches(COMMAND_DEFINITIONS, 'pol').map((c) => c.name)).toEqual(['poll']);
  });

  // The built-ins custom commands always yielded to still win.
  it('a custom /ban never fires', async () => {
    appState.settings.chat_commands.user_commands = [
      { id: 'u3', trigger: 'ban', expansion: 'banana', enabled: true, require_slash: true },
    ];
    await run('/ban bob');
    expect(send).not.toHaveBeenCalled();
    expect(calls('ban_user')).toHaveLength(1);
  });
});

describe('platform differences', () => {
  it('/lurk on Kick or YouTube sends words, not a literal /me', async () => {
    const kickSend = vi.fn();
    await runExtendedCommand({
      command: 'lurk',
      typed: 'lurk',
      args: [],
      rest: '',
      broadcasterId: '',
      broadcasterLogin: 'kick:somechan',
      send: kickSend,
      dispatch: async () => true,
    });
    expect(calls('slash_lurk_line')[0]).toEqual({ note: null, action: false });
    expect(kickSend).toHaveBeenCalledWith('ok');
    await run('/lurk making dinner');
    expect(calls('slash_lurk_line')[1]).toEqual({ note: 'making dinner', action: true });
    expect(send).toHaveBeenCalledWith('ok');
  });

  it('/help on the phone leaves out desktop-only commands', () => {
    const phone = helpLines(undefined, true).join(' ');
    const desktop = helpLines(undefined, false).join(' ');
    expect(desktop).toContain('/proc');
    expect(phone).not.toContain('/proc');
    expect(phone).not.toContain('/screenshot');
    expect(phone).toContain('/spam');
    expect(helpLines('proc', true)).toEqual(['/proc works in the desktop app.']);
  });
});
