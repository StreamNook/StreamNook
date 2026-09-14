// Streamer mode, mirrored from Rust (services/streamer_mode.rs). Rust owns
// detection and the flag; every window reads it through this tiny store and
// the `streamer-mode-changed` event. Consumers: viewer counts, highlight
// sounds, link previews, restricted-user rows.

import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from './logger';

export interface StreamerModeState {
  active: boolean;
  mode: 'off' | 'on' | 'auto';
}

interface Store extends StreamerModeState {
  started: boolean;
  start: () => void;
}

export const useStreamerMode = create<Store>((set, get) => ({
  active: false,
  mode: 'off',
  started: false,
  start: () => {
    if (get().started) return;
    set({ started: true });
    void invoke<StreamerModeState>('get_streamer_mode_state')
      .then((s) => set({ active: s.active, mode: s.mode }))
      .catch((err) => Logger.debug('[StreamerMode] initial state failed:', err));
    void listen<StreamerModeState>('streamer-mode-changed', (e) => {
      set({ active: e.payload.active, mode: e.payload.mode });
    }).catch((err) => Logger.debug('[StreamerMode] listen failed:', err));
  },
}));

/** Non-reactive read for utilities (sound throttle, etc). */
export function isStreamerModeActive(): boolean {
  return useStreamerMode.getState().active;
}
