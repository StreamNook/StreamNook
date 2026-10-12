// Why Twitch is signed out, when StreamNook signed the user out itself, and the
// one prompt that says so.
//
// Rust holds the reason (services/twitch_signin.rs) and carries it on the
// Twitch row of the account roster, so every window reads the same answer and
// it survives a relaunch. This file only words it and keeps the prompt in step
// with it: the prompt appears when the reason does and goes when a sign-in
// clears it.

import { useAccountRosterStore, type AccountRoster, type SignInReason } from '../stores/accountRosterStore';

const REASON_TEXT: Record<SignInReason, { sentence: string; short: string }> = {
  new_permissions: {
    sentence: 'Twitch needs you to sign in again for new features.',
    short: 'Sign in again for new features',
  },
};

/** The reason Twitch is waiting on a new sign-in, or null when none is owed. */
export function twitchSignInReason(roster: AccountRoster | null | undefined): SignInReason | null {
  const twitch = roster?.main.find((a) => a.id === 'twitch');
  if (!twitch || twitch.status === 'connected') return null;
  return twitch.reason ?? null;
}

/** Spoken to the viewer: a full sentence, or the `short` line for a row that
 *  already says "Twitch". */
export function signInReasonText(reason: SignInReason, form: 'sentence' | 'short' = 'sentence'): string {
  return REASON_TEXT[reason][form];
}

/** The pending reason, for surfaces that would otherwise hit a signed-out wall. */
export function useTwitchSignInReason(): SignInReason | null {
  return useAccountRosterStore((s) => twitchSignInReason(s.roster));
}

export interface SignInPromptDeps {
  /** Whether this window still thinks it is signed in. */
  isSignedIn: () => boolean;
  /** Drop this window's signed-in view: the token behind it is already gone. */
  markSignedOut: () => void;
  /** Show the prompt; returns a handle for taking it down, if one was shown. */
  show: (text: string) => number | undefined;
  hide: (handle: number) => void;
}

/**
 * Feed it the pending reason each time the roster changes. It shows the prompt
 * once per reason (dismissing it does not bring it back this session), and
 * takes it down when the reason clears.
 */
export function createSignInPrompt(deps: SignInPromptDeps): (reason: SignInReason | null) => void {
  let shownFor: SignInReason | null = null;
  let handle: number | undefined;
  return (reason) => {
    if (!reason) {
      if (handle !== undefined) deps.hide(handle);
      handle = undefined;
      shownFor = null;
      return;
    }
    // A profile read can land after the token was cleared; the roster is the
    // truth, so the window follows it.
    if (deps.isSignedIn()) deps.markSignedOut();
    if (shownFor === reason) return;
    shownFor = reason;
    handle = deps.show(signInReasonText(reason));
  };
}
