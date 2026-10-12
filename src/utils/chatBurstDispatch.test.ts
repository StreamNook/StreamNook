// Run with: npm test
//
// /spam and /pyramid are parsed in Rust (services/chat_burst.rs, where the
// count, the --delay flag and stop are tested). The chat box only hands the
// text after the command word to `run_chat_burst`, spacing intact, and shows
// what comes back. This pins that hand-off.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
const addToast = vi.fn();
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ addToast, settings: {} }) },
}));

import { handleSlashCommand } from './commandHandler';
import { COMMAND_DEFINITIONS, RESERVED_TRIGGERS } from './chatCommands';

describe('/spam and /pyramid dispatch', () => {
  beforeEach(() => {
    invoke.mockReset();
    addToast.mockReset();
  });

  it('are listed built-in commands the settings page flags a custom command against', () => {
    for (const name of ['spam', 'pyramid']) {
      const def = COMMAND_DEFINITIONS.find((c) => c.name === name);
      expect(def?.category).toBe('Everyone');
      expect(def?.details).toMatch(/time you out for spam/);
      expect(RESERVED_TRIGGERS.has(name)).toBe(true);
    }
  });

  it('hands the raw arguments to Rust with the channel', async () => {
    invoke.mockResolvedValue({ message: 'Spam started', status: null });
    const sent = vi.fn();
    const handled = await handleSlashCommand('/SPAM 20  two  spaces --delay 200', '123', 'somechan', sent);
    expect(handled).toBe(true);
    expect(sent).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledWith('run_chat_burst', {
      kind: 'spam',
      args: ' 20  two  spaces --delay 200',
      channelLogin: 'somechan',
      channelId: '123',
    });
  });

  it('routes stop the same way and shows a refusal as an error toast', async () => {
    invoke.mockRejectedValue('A spam is already running here (3/20). /spam stop ends it');
    await handleSlashCommand('/pyramid stop', '1', 'c', vi.fn());
    expect(invoke).toHaveBeenCalledWith('run_chat_burst', expect.objectContaining({ kind: 'pyramid', args: ' stop' }));
    expect(addToast).toHaveBeenCalledWith('A spam is already running here (3/20). /spam stop ends it', 'error');
  });
});
