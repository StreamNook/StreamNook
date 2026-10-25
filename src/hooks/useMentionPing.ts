import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { playSound } from '../utils/notificationSound';

interface MentionPing {
  channel: string;
  sound: string;
  /** Percent of the sound's own level. */
  volume: number;
}

/** Rust's event that plays the sound, sent to one window. */
const PING_EVENT = 'chat-mention-ping';
/** Tells Rust whether this window can play it. */
const READY_COMMAND = 'mention_ping_ready';

/**
 * Plays the mention sound Rust sends this window, and tells Rust while it
 * can. Returns the stop. Never throws: outside the app it does nothing.
 */
export function startMentionPing(): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  const say = (ready: boolean) => {
    void invoke(READY_COMMAND, { ready }).catch(() => {});
  };
  const gone = () => say(false);
  void Promise.resolve()
    .then(() => {
      // Window-scoped: a plain `listen` hears events sent to every other window too.
      return getCurrentWebviewWindow().listen<MentionPing>(PING_EVENT, (e) =>
        playSound(e.payload.sound, e.payload.volume),
      );
    })
    .then((fn) => {
      if (stopped) {
        fn();
        return;
      }
      unlisten = fn;
      say(true);
      if (typeof window !== 'undefined') window.addEventListener('pagehide', gone);
    })
    .catch(() => {});
  return () => {
    stopped = true;
    if (typeof window !== 'undefined') window.removeEventListener('pagehide', gone);
    if (unlisten) {
      unlisten();
      say(false);
    }
  };
}

/**
 * Plays the mention sound Rust asks for. Rust decides when (the setting,
 * streamer mode, cooldowns, message age) across every joined chat, hidden and
 * docked ones included, and which ONE window plays it: main while its page
 * listens, else a MultiChat window, else a chat overlay. This window only
 * plays what the event names.
 *
 * Mount once per window, in main, MultiChat and the chat overlay.
 */
export function useMentionPing(): void {
  useEffect(() => startMentionPing(), []);
}
