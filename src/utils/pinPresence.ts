// Starts this window's side of the pins' presence rules (SN-284): while Pin
// mentions is on, the window reports its input and its pin surfaces to Rust;
// while it is off, nothing is sent, no listener is installed and no event is
// heard. Called once per window page from main.tsx.

import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { setInputSideEffect, setPinInputEnabled } from './pinInput';
import { pinSurfaces } from './pinSurfaces';
import { IS_MOBILE } from './platform';

/** Rust's `mention_pins` event: what the pins draw changed (no payload). */
export const MENTION_PINS_CHANGED_EVENT = 'mention-pins-changed';

let started = false;

const pinsOn = (): boolean => useAppStore.getState().settings?.chat_design?.pin_mentions === true;

/**
 * `surfaces: false` for a window that has chat stacks of no kind (the emote
 * palette): it still reports input, which Rust credits to the window it serves.
 */
export function startPinPresence(opts: { surfaces?: boolean } = {}): void {
  if (started) return;
  started = true;
  const withSurfaces = opts.surfaces !== false;
  let unlisten: (() => void) | null = null;

  const resend = () => pinSurfaces.resend();
  const onVisibility = () => {
    if (document.visibilityState === 'visible') resend();
  };

  const apply = (on: boolean) => {
    setPinInputEnabled(on);
    if (!withSurfaces) return;
    pinSurfaces.setEnabled(on);
    if (on) {
      window.addEventListener('focus', resend);
      document.addEventListener('visibilitychange', onVisibility);
      void listen(MENTION_PINS_CHANGED_EVENT, resend).then((off) => {
        if (!pinSurfaces.isEnabled()) off();
        else unlisten = off;
      });
    } else {
      window.removeEventListener('focus', resend);
      document.removeEventListener('visibilitychange', onVisibility);
      unlisten?.();
      unlisten = null;
    }
  };

  if (IS_MOBILE) {
    // A failed first `set_app_interactive` report is retried on the next input.
    void import('../mobile/lifecycle').then((m) => setInputSideEffect(m.retryInteractiveReport));
  }

  let last = pinsOn();
  apply(last);
  useAppStore.subscribe(() => {
    const on = pinsOn();
    if (on === last) return;
    last = on;
    apply(on);
  });
}
