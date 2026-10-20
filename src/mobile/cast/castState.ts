// The page's read-only mirror of Rust's cast session (services/cast_session.rs).
//
// Rust owns every decision: what the TV plays, following raids and switches,
// retrying a stream the TV refused. This module only holds the last state Rust
// emitted so components can render it, and forwards the viewer's taps.
import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { TwitchStream } from '../../types';
import { streamProvider } from '../../utils/streamProvider';
import { Logger } from '../../utils/logger';

export interface CastTarget {
  provider: string;
  login: string;
  name: string;
  title: string;
}

export interface CastState {
  /** Whether this build offers Cast and mirroring at all (Rust `cast_session::OFFERED`). */
  offered: boolean;
  available: boolean;
  phase: 'idle' | 'connecting' | 'casting';
  device: string | null;
  audioOnly: boolean;
  target: CastTarget | null;
  player: 'loading' | 'playing' | 'paused' | 'buffering' | 'idle';
  error: string | null;
}

const IDLE: CastState = {
  offered: false,
  available: false,
  phase: 'idle',
  device: null,
  audioOnly: false,
  target: null,
  player: 'idle',
  error: null,
};

let current: CastState = IDLE;
const subscribers = new Set<() => void>();
let started = false;

function publish(next: CastState): void {
  current = next;
  for (const fn of subscribers) fn();
}

function start(): void {
  if (started) return;
  started = true;
  void listen<CastState>('cast://state', (e) => publish(e.payload)).catch((e) =>
    Logger.warn('[Cast] could not listen for cast state:', e),
  );
  void invoke<CastState>('cast_get_state')
    .then(publish)
    .catch((e) => Logger.warn('[Cast] could not read cast state:', e));
}

function subscribe(fn: () => void): () => void {
  start();
  subscribers.add(fn);
  return () => subscribers.delete(fn);
}

export function useCastState(): CastState {
  return useSyncExternalStore(subscribe, () => current);
}

/** Platforms whose streams a Cast device can fetch on its own. */
export function isCastable(stream: TwitchStream | null): boolean {
  const provider = streamProvider(stream);
  return provider === 'twitch' || provider === 'kick';
}

/**
 * A device is connected and the stream on screen is one it plays. Connecting
 * does not count: a connection that fails must leave the phone's player alone.
 */
export function castOwnsStream(state: CastState, stream: TwitchStream | null): boolean {
  return state.phase === 'casting' && !!stream && isCastable(stream);
}

export function openCastPicker(): Promise<void> {
  return invoke('cast_open_picker');
}

export function castControl(action: 'play' | 'pause' | 'stop'): Promise<void> {
  return invoke('cast_control', { action });
}

/** Opens the system's screen-cast page; false when this phone has none. */
export function openMirroring(): Promise<boolean> {
  return invoke<boolean>('cast_open_mirroring');
}
