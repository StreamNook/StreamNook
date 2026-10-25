import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { playSound, playSoundFile } from '../utils/notificationSound';

interface MentionPing {
  /** What this page claims before playing. */
  id: number;
  channel: string;
  sound: string;
  /** Percent of the sound's own level. */
  volume: number;
  /** A custom sound's file, so this window needs no settings to find it. */
  path?: string;
}

/** Rust's event that plays the sound, sent to every window that could play it. */
const PING_EVENT = 'chat-mention-ping';
/** True for the first page to claim a ping: that page plays it. */
const CLAIM_COMMAND = 'mention_ping_claim';

/**
 * Plays the mention pings Rust gives this window. Returns the stop. Never
 * throws: outside the app it does nothing.
 */
export function startMentionPing(): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  void Promise.resolve()
    .then(() =>
      // Window-scoped: a plain `listen` hears events sent to every other window too.
      getCurrentWebviewWindow().listen<MentionPing>(PING_EVENT, (e) => {
        const { id, sound, volume, path } = e.payload;
        void invoke<boolean>(CLAIM_COMMAND, { id })
          .then((mine) => {
            if (!mine) return;
            if (path) playSoundFile(sound, path, volume);
            else playSound(sound, volume);
          })
          .catch(() => {});
      }),
    )
    .then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    stopped = true;
    unlisten?.();
  };
}

/**
 * Plays the mention sound Rust asks for. Rust decides when (the setting,
 * streamer mode, cooldowns, message age) across every joined chat, hidden and
 * docked ones included, and sends each ping to every window that could play
 * it: main, MultiChat and the chat overlay. The first page to claim it plays
 * it, so it plays once in whichever of them is alive and listening.
 *
 * Mount once per window, in main, MultiChat and the chat overlay.
 */
export function useMentionPing(): void {
  useEffect(() => startMentionPing(), []);
}
