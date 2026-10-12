import { useEffect } from 'react';
import { useAppStore } from '../stores/AppStore';
import { useAccountRosterStore } from '../stores/accountRosterStore';
import { createSignInPrompt, twitchSignInReason } from '../utils/twitchSignIn';
import { IS_MOBILE } from '../utils/platform';

/**
 * Tells the viewer, until they act on it, that Twitch needs them to sign in
 * again, whenever Rust says so (the reason on the roster's Twitch row). The
 * toast stays until it is dismissed or a sign-in clears the reason; the title
 * bar and the signed-out walls keep saying it either way.
 *
 * Mount ONCE, in the main window (and the phone shell): one prompt per app, not
 * one per window.
 */

// Module scope, so a remount never shows the same prompt twice.
let prompt: ReturnType<typeof createSignInPrompt> | null = null;

function signInPrompt() {
  prompt ??= createSignInPrompt({
    isSignedIn: () => useAppStore.getState().isAuthenticated,
    markSignedOut: () => useAppStore.setState({ isAuthenticated: false, currentUser: null, followedStreams: [] }),
    show: (text) => {
      // The phone's signed-out screen is the prompt there, and it says why.
      if (IS_MOBILE) return undefined;
      const id = Date.now() + Math.random();
      useAppStore.getState().addToast(
        text,
        'warning',
        { label: 'Sign in', onClick: () => void useAppStore.getState().loginToTwitch() },
        { sticky: true, id },
      );
      return id;
    },
    hide: (id) => useAppStore.getState().removeToast(id),
  });
  return prompt;
}

export function useTwitchSignInPrompt(): void {
  useEffect(() => {
    const apply = signInPrompt();
    apply(twitchSignInReason(useAccountRosterStore.getState().roster));
    return useAccountRosterStore.subscribe((state, prev) => {
      if (state.roster !== prev.roster) apply(twitchSignInReason(state.roster));
    });
  }, []);
}
