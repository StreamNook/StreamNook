// The page's read-only mirror of TV screen mode (services/tv_screen.rs): while
// the phone mirrors to a TV, the stream plays full-frame on the TV. Rust decides
// when; this holds the last state it emitted and forwards the viewer's taps.
import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../../utils/logger';

export interface TvScreenState {
  display: { name: string; width: number; height: number } | null;
  enabled: boolean;
  active: boolean;
  player: 'loading' | 'playing' | 'paused' | 'buffering' | 'idle';
  error: string | null;
}

let current: TvScreenState = { display: null, enabled: true, active: false, player: 'idle', error: null };
const subscribers = new Set<() => void>();
let started = false;

function publish(next: TvScreenState): void {
  current = next;
  for (const fn of subscribers) fn();
}

function start(): void {
  if (started) return;
  started = true;
  void listen<TvScreenState>('tv-screen://state', (e) => publish(e.payload)).catch((e) =>
    Logger.warn('[TvScreen] could not listen for state:', e),
  );
  void invoke<TvScreenState>('tv_get_state')
    .then(publish)
    .catch((e) => Logger.warn('[TvScreen] could not read state:', e));
}

function subscribe(fn: () => void): () => void {
  start();
  subscribers.add(fn);
  return () => subscribers.delete(fn);
}

export function useTvScreenState(): TvScreenState {
  return useSyncExternalStore(subscribe, () => current);
}

export function setTvScreenEnabled(enabled: boolean): Promise<void> {
  return invoke('tv_set_enabled', { enabled });
}

export function tvControl(action: 'play' | 'pause'): Promise<void> {
  return invoke('tv_control', { action });
}
