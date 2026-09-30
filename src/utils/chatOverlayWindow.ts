// Opens the transparent always-on-top chat overlay window (`#/chat-overlay`).
//
// Rust builds and places the window (open_chat_overlay in
// commands/popout_window.rs): one overlay per channel, at most four, placed
// in physical pixels and kept wholly on screen. A second open for a channel
// brings its overlay forward.
//
// Windows note: an exclusive-fullscreen game hides every topmost window; the
// overlay only floats over borderless-windowed games.

import { invoke } from '@tauri-apps/api/core';

export interface OpenChatOverlayOptions {
  channel: string;
  channelId?: string | null;
  channelName?: string | null;
}

export type PopoutOutcome = 'opened' | 'focused' | 'limit_reached';

export async function openChatOverlayWindow(options: OpenChatOverlayOptions): Promise<PopoutOutcome> {
  const outcome = await invoke<PopoutOutcome>('open_chat_overlay', {
    channel: options.channel,
    channelId: options.channelId ?? null,
    channelName: options.channelName ?? null,
  });
  if (outcome === 'limit_reached') {
    const { useAppStore } = await import('../stores/AppStore');
    useAppStore.getState().addToast('Four chat overlays are open. Close one to float another.', 'info');
  }
  return outcome;
}
