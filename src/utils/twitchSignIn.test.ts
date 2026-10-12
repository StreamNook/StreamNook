import { describe, expect, it, vi } from 'vitest';
import type { AccountRoster, RosterAccount, MainAccountId } from '../stores/accountRosterStore';
import { createSignInPrompt, signInReasonText, twitchSignInReason, type SignInPromptDeps } from './twitchSignIn';

const row = (id: MainAccountId, over: Partial<RosterAccount<MainAccountId>> = {}): RosterAccount<MainAccountId> => ({
  id,
  status: 'not_connected',
  name: null,
  handle: null,
  avatar_url: null,
  account_id: null,
  reason: null,
  ...over,
});

const roster = (twitch: Partial<RosterAccount<MainAccountId>>): AccountRoster => ({
  main: [row('twitch', twitch), row('youtube'), row('kick'), row('tiktok')],
  extras: [],
});

describe('twitchSignInReason', () => {
  it('reads the reason Rust put on the Twitch row', () => {
    expect(twitchSignInReason(roster({ status: 'expired', reason: 'new_permissions' }))).toBe('new_permissions');
  });

  it('owes nothing when signed in, signed out by choice, or before the roster lands', () => {
    expect(twitchSignInReason(roster({ status: 'connected', name: 'Viewer' }))).toBeNull();
    expect(twitchSignInReason(roster({ status: 'not_connected' }))).toBeNull();
    expect(twitchSignInReason(roster({ status: 'expired' }))).toBeNull();
    expect(twitchSignInReason(null)).toBeNull();
    expect(twitchSignInReason(undefined)).toBeNull();
  });

  it('never reports a reason on a connected row', () => {
    expect(twitchSignInReason(roster({ status: 'connected', reason: 'new_permissions' }))).toBeNull();
  });

  it('reads an older roster that has no reason field at all', () => {
    const old = roster({ status: 'not_connected' });
    delete old.main[0].reason;
    expect(twitchSignInReason(old)).toBeNull();
  });
});

describe('signInReasonText', () => {
  it('speaks to the viewer, short, without dashes', () => {
    const sentence = signInReasonText('new_permissions');
    expect(sentence).toBe('Twitch needs you to sign in again for new features.');
    expect(signInReasonText('new_permissions', 'short')).toBe('Sign in again for new features');
    for (const text of [sentence, signInReasonText('new_permissions', 'short')]) {
      // En and em dash, by code point so this file holds neither.
      for (const dash of [0x2013, 0x2014]) expect(text).not.toContain(String.fromCharCode(dash));
    }
  });
});

describe('createSignInPrompt', () => {
  const setup = (signedIn = true) => {
    let isSignedIn = signedIn;
    let next = 1;
    const deps = {
      isSignedIn: vi.fn(() => isSignedIn),
      markSignedOut: vi.fn(() => {
        isSignedIn = false;
      }),
      show: vi.fn((): number | undefined => next++),
      hide: vi.fn(),
    } satisfies SignInPromptDeps;
    return { deps, prompt: createSignInPrompt(deps), signIn: () => (isSignedIn = true) };
  };

  it('shows the prompt once and signs the window out when a reason appears', () => {
    const { deps, prompt } = setup(true);
    prompt('new_permissions');
    expect(deps.markSignedOut).toHaveBeenCalledTimes(1);
    expect(deps.show).toHaveBeenCalledWith('Twitch needs you to sign in again for new features.');

    // Every later roster push with the same reason: no second toast.
    prompt('new_permissions');
    prompt('new_permissions');
    expect(deps.show).toHaveBeenCalledTimes(1);
    expect(deps.hide).not.toHaveBeenCalled();
  });

  it('takes the prompt down when a sign-in clears the reason', () => {
    const { deps, prompt, signIn } = setup(false);
    prompt('new_permissions');
    expect(deps.markSignedOut).not.toHaveBeenCalled();
    signIn();
    prompt(null);
    expect(deps.hide).toHaveBeenCalledWith(1);
    // Signing in must not be undone by the cleared roster.
    expect(deps.markSignedOut).not.toHaveBeenCalled();
    // Nothing left to take down.
    prompt(null);
    expect(deps.hide).toHaveBeenCalledTimes(1);
  });

  it('keeps the window signed out if a stale profile read signs it back in', () => {
    const { deps, prompt, signIn } = setup(false);
    prompt('new_permissions');
    signIn();
    prompt('new_permissions');
    expect(deps.markSignedOut).toHaveBeenCalledTimes(1);
    expect(deps.show).toHaveBeenCalledTimes(1);
  });

  it('asks again if the reason returns after a sign-in', () => {
    const { deps, prompt } = setup(false);
    prompt('new_permissions');
    prompt(null);
    prompt('new_permissions');
    expect(deps.show).toHaveBeenCalledTimes(2);
  });

  it('copes with a prompt that was not shown', () => {
    const { deps, prompt } = setup(false);
    deps.show.mockReturnValueOnce(undefined);
    prompt('new_permissions');
    prompt(null);
    expect(deps.hide).not.toHaveBeenCalled();
  });
});
