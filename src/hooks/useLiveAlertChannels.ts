// The followed channels with their live-alert state, for the Which channels
// line and list. Rust builds it (Twitch follows, other platforms' follows and
// favorites, each with `on`) and counts the mutes that match a listed channel
// (`muted_count`, null when the follows could not be read). Read again when
// the stored mute list changes.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import type { TwitchStream } from '../types';
import type { CopyAnswer } from '../utils/liveAlerts';

export interface AlertListChannel extends TwitchStream {
  provider: NonNullable<TwitchStream['provider']>;
  on: boolean | null;
}

export interface AlertChannelList {
  channels: AlertListChannel[];
  truncated: boolean;
  muted_count: number | null;
  /** The last Copy from Twitch this session while it can still be undone
   *  (Rust keeps it 10 min, until the next copy or an account change). */
  last_copy: CopyAnswer | null;
}

export interface AlertChannelsState {
  list: AlertChannelList | null;
  failed: boolean;
}

export function useLiveAlertChannels(active: boolean): AlertChannelsState {
  const mutedKey = useAppStore((s) => (s.settings.live_notifications?.muted_live_channels ?? []).join('\n'));
  const [state, setState] = useState<AlertChannelsState>({ list: null, failed: false });

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    invoke<AlertChannelList>('get_followed_channels_for_alerts', { allPlatforms: true })
      .then((list) => {
        if (!cancelled) setState({ list, failed: false });
      })
      .catch((err) => {
        Logger.warn('[LiveAlertChannels] list read failed:', err);
        if (!cancelled) setState((s) => ({ list: s.list, failed: true }));
      });
    return () => {
      cancelled = true;
    };
  }, [active, mutedKey]);

  return state;
}
