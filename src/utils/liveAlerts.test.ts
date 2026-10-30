import { beforeEach, describe, expect, it, vi } from 'vitest';

const mock = vi.hoisted(() => ({
  calls: [] as Array<{ cmd: string; args: Record<string, unknown> }>,
  release: [] as Array<() => void>,
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) => {
    mock.calls.push({ cmd, args });
    return new Promise((resolve) => {
      mock.release.push(() => resolve(args.on));
    });
  },
}));

import {
  copyErrorText,
  copyResultText,
  setLiveAlert,
  sheetIntro,
  undoResultText,
  whichChannelsDetail,
} from './liveAlerts';

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('setLiveAlert', () => {
  beforeEach(() => {
    mock.calls.length = 0;
    mock.release.length = 0;
  });

  it('sends the second press only after the first answered, in press order', async () => {
    const first = setLiveAlert({ provider: 'twitch', userId: '1', userLogin: 'alpha' }, false);
    const second = setLiveAlert({ provider: 'twitch', userId: '1', userLogin: 'alpha' }, true);
    await flush();
    expect(mock.calls.map((c) => c.args.on)).toEqual([false]);
    mock.release[0]();
    await expect(first).resolves.toBe(false);
    await flush();
    expect(mock.calls.map((c) => c.args.on)).toEqual([false, true]);
    mock.release[1]();
    await expect(second).resolves.toBe(true);
  });

  it('sends the state to set, with empty strings for missing ids', async () => {
    const p = setLiveAlert({ provider: 'kick', userLogin: 'beta' }, false);
    await flush();
    expect(mock.calls[0]).toEqual({
      cmd: 'set_live_alert',
      args: { provider: 'kick', userId: '', userLogin: 'beta', on: false },
    });
    mock.release[0]();
    await p;
  });
});

describe('copy text', () => {
  const base = { copy_id: 1, added: [], bells_off: 0, already: 0, skipped: 0, truncated: false };

  it('counts what was muted', () => {
    expect(copyResultText({ ...base, added: ['a', 'b'], bells_off: 2 })).toBe('Muted 2 channels whose Twitch bell is off.');
    expect(copyResultText({ ...base, added: ['a'], bells_off: 1 })).toBe('Muted 1 channel whose Twitch bell is off.');
  });

  it('says when nothing was new, in the phones words', () => {
    expect(copyResultText({ ...base, bells_off: 3, already: 3 })).toBe(
      'Nothing new to copy. Every channel whose Twitch bell is off is already muted.',
    );
    expect(copyResultText(base)).toBe('Nothing new to copy. Every channel whose Twitch bell is off is already muted.');
  });

  it('says when only the first 2,000 follows were read', () => {
    expect(copyResultText({ ...base, added: ['a'], bells_off: 1, truncated: true })).toBe(
      'Muted 1 channel whose Twitch bell is off. Only your first 2,000 follows were checked.',
    );
  });

  it('words every error code and falls back for unknown ones', () => {
    expect(copyErrorText('signed-out')).toBe('Sign in to Twitch to copy your bells.');
    expect(copyErrorText('wrong-account')).toBe(
      'Twitch is signed in as another account here. Switch back to copy your bells.',
    );
    expect(copyErrorText('busy')).toBe('Already copying your bells.');
    expect(copyErrorText('stale')).toBe("That copy can't be undone anymore.");
    expect(copyErrorText('save-failed')).toBe("Couldn't save that. Try again.");
    expect(copyErrorText(new Error('x'))).toBe("Twitch didn't answer. Try again in a moment.");
  });

  it('words an undo as the phones do', () => {
    expect(undoResultText(0)).toBe('Nothing left to undo.');
    expect(undoResultText(1)).toBe('Alerts are back on for 1 channel.');
    expect(undoResultText(2000)).toBe('Alerts are back on for 2,000 channels.');
  });
});

describe('Which channels words', () => {
  it('counts only muted channels you follow once Rust answers', () => {
    expect(whichChannelsDetail(3, 9)).toBe('Everything you follow except 3');
    expect(whichChannelsDetail(0, 9)).toBe('Everything you follow');
  });

  it('falls back to the stored list while not read, and when the list failed', () => {
    expect(whichChannelsDetail(undefined, 4)).toBe('Everything you follow except 4');
    expect(whichChannelsDetail(null, 1)).toBe('1 channel muted');
    expect(whichChannelsDetail(null, 0)).toBe('Everything you follow');
  });

  it('opens the list with the same numbers', () => {
    expect(sheetIntro(2, 5)).toBe('Alerting for every channel you follow except 2.');
    expect(sheetIntro(0, 5)).toBe('Alerting for every channel you follow. Turn off any you would rather not hear about.');
    expect(sheetIntro(null, 2)).toBe('2 channels are muted.');
  });
});
