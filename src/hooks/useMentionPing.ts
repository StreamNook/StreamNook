import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { playSound } from '../utils/notificationSound';

interface MentionPing {
  channel: string;
  sound: string;
  /** Percent of the sound's own level. */
  volume: number;
}

/**
 * Plays the mention sound Rust asks for. Rust decides when (the setting,
 * streamer mode, cooldowns, message age) across every joined chat, hidden and
 * docked ones included; this window only plays what the event names.
 *
 * Mount ONCE, in the main window: the event is sent to it alone.
 */
export function useMentionPing(): void {
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<MentionPing>('chat-mention-ping', (e) => playSound(e.payload.sound, e.payload.volume)).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
}
