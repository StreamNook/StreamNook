// Hype-train banners, fed by Rust (services/hype_train_watch.rs): a surface says
// which channels it shows, and one poll per channel serves every surface in
// every window.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { HypeTrainData } from '../types';
import { useActivityStore } from '../stores/activityStore';
import { makeKey } from '../utils/providerKey';
import { Logger } from '../utils/logger';

export interface HypeTrainSource {
  login: string;
  /** Broadcaster id when known; Rust looks it up otherwise. */
  channelId?: string;
  name?: string;
}

interface HypeTrainUpdate {
  login: string;
  /** The train started, or reached a new level, since the previous update. */
  level_changed: boolean;
  train: HypeTrainData | null;
}

/**
 * Show these channels' hype trains. `onUpdate` receives each change (null when
 * a train ends). Returns the cleanup that stops showing them.
 */
export function watchHypeTrains(
  sources: HypeTrainSource[],
  onUpdate: (login: string, train: HypeTrainData | null, levelChanged: boolean) => void,
): () => void {
  const logins = new Set(sources.map((s) => s.login.toLowerCase()));
  let stopped = false;
  let unlisten: (() => void) | undefined;
  void listen<HypeTrainUpdate>('hype-train://update', (event) => {
    const { login, train, level_changed } = event.payload;
    if (!stopped && logins.has(login)) onUpdate(login, train, level_changed);
  }).then((fn) => {
    if (stopped) fn();
    else unlisten = fn;
  });
  for (const s of sources) {
    invoke('hype_train_watch', { login: s.login, channelId: s.channelId || null, name: s.name || null }).catch(
      (err) => Logger.warn('[HypeTrain] watch failed:', err),
    );
  }
  return () => {
    stopped = true;
    unlisten?.();
    for (const login of logins) invoke('hype_train_unwatch', { login }).catch(() => {});
  };
}

/** A train's start or level-up, as a row in the MultiChat activity feed. The
 *  per-train, per-level id dedupes, so two surfaces reporting it add one row. */
export function recordHypeTrainActivity(login: string, displayName: string, train: HypeTrainData): void {
  useActivityStore.getState().addEvent({
    id: `hype-${login}-${train.id || 'train'}-L${train.level}`,
    timestamp: new Date().toISOString(),
    provider: 'twitch',
    channel: makeKey('twitch', login),
    channel_display: displayName || login,
    kind: 'hypetrain',
    actor: { username: login, display_name: displayName || login },
    system_text: train.is_golden_kappa ? `Level ${train.level} · Golden Kappa` : `Level ${train.level}`,
  });
}
