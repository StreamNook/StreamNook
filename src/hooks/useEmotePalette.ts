// A chat box's side of the popped-out emote menu. Rust owns which chat box the
// palette types into (commands/emote_palette.rs); this asks for the palette,
// keeps it on this chat box's channel, and puts what the palette sends at the
// cursor. Nothing listens or runs until the viewer pops the menu out.
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { IS_MOBILE } from '../utils/platform';
import { Logger } from '../utils/logger';

/** The chat the palette serves, in Rust's key names (`PaletteChannel`). */
export interface PaletteChannel {
  /** Twitch login, Kick slug or YouTube video id: what the emote set is fetched by. */
  login: string;
  id?: string | null;
  name?: string | null;
  provider: 'twitch' | 'kick' | 'youtube';
}

interface InsertEvent {
  composer: string;
  text: string;
}

/**
 * `channel` is this chat box's channel, or null when it has none the menu can
 * serve. `insert` puts text at the cursor; it is read through a ref, so it may
 * change every render. Returns the pop-out action, or undefined where there is
 * nothing to pop out to (a phone, a channel without emotes).
 */
export function useEmotePalette(
  channel: PaletteChannel | null,
  insert: (text: string) => void,
): (() => void) | undefined {
  const composer = useId();
  const [served, setServed] = useState(false);
  const insertRef = useRef(insert);
  useEffect(() => {
    insertRef.current = insert;
  });

  // Keyed by value, so a new object for the same channel changes nothing.
  const channelKey = channel ? `${channel.provider}:${channel.login}:${channel.id ?? ''}:${channel.name ?? ''}` : '';
  const channelRef = useRef(channel);
  useEffect(() => {
    channelRef.current = channel;
  });

  const popOut = useCallback(() => {
    const current = channelRef.current;
    if (!current) return;
    setServed(true);
    invoke('open_emote_palette', { composer, channel: current }).catch((e) =>
      Logger.warn('[EmotePalette] could not open the emote menu:', e),
    );
  }, [composer]);

  // Text from the palette. Every chat box in this window hears the event, so
  // each keeps only its own (Rust also sends it to this window only).
  useEffect(() => {
    if (!served) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWindow()
      .listen<InsertEvent>('emote-palette://insert', (e) => {
        if (e.payload.composer === composer) insertRef.current(e.payload.text);
      })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [served, composer]);

  // This chat box changed channels: the palette follows. Rust ignores it
  // unless this chat box is the one the palette serves.
  useEffect(() => {
    if (!served || !channelKey) return;
    void invoke('update_emote_palette_channel', { composer, channel: channelRef.current }).catch(() => {});
  }, [served, composer, channelKey]);

  // The chat box is going away: its palette closes with it.
  useEffect(() => {
    if (!served) return;
    return () => {
      void invoke('release_emote_palette', { composer }).catch(() => {});
    };
  }, [served, composer]);

  if (IS_MOBILE || !channel) return undefined;
  return popOut;
}
