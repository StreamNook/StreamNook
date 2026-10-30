// One channel's live alerts, for a bell or a menu row.
//
// Asks Rust on mount and again whenever the stored mute list changes, so
// every surface shows the same state within one settings refresh. Answers
// pass through `liveAlertReduce`, which drops any answer older than the
// newest press or read.

import { useEffect, useReducer, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import { LIVE_ALERT_INITIAL, liveAlertReduce, shownFor } from '../utils/liveAlertView';
import { setLiveAlert, type AlertChannelRef } from '../utils/liveAlerts';

export interface LiveAlert {
  /** true on, false off, null hidden (no identity, or not known yet). */
  on: boolean | null;
  busy: boolean;
  setOn: (on: boolean) => void;
}

/** `enabled` false skips the read (a quiet control that wakes on reach). */
export function useLiveAlert(channel: AlertChannelRef, enabled = true): LiveAlert {
  const provider = channel.provider;
  const userId = channel.userId ?? '';
  const userLogin = channel.userLogin ?? '';
  const key = `${provider}|${userId}|${userLogin}`;
  const mutedKey = useAppStore((s) => (s.settings.live_notifications?.muted_live_channels ?? []).join('\n'));
  const [view, dispatch] = useReducer(liveAlertReduce, LIVE_ALERT_INITIAL);
  const counter = useRef(0);

  useEffect(() => {
    if (!enabled || !provider || (!userId && !userLogin)) return;
    counter.current += 1;
    const id = counter.current;
    let cancelled = false;
    invoke<boolean | null>('live_alert_state', { provider, userId, userLogin })
      .then((on) => {
        if (!cancelled) dispatch({ kind: 'answer', key, id, on });
      })
      .catch((err) => Logger.warn('[LiveAlert] state read failed:', err));
    return () => {
      cancelled = true;
    };
  }, [enabled, provider, userId, userLogin, key, mutedKey]);

  const setOn = (on: boolean) => {
    counter.current += 1;
    const id = counter.current;
    dispatch({ kind: 'press', key, id, on });
    setLiveAlert({ provider, userId, userLogin }, on)
      .then((answer) => dispatch({ kind: 'answer', key, id, on: answer }))
      .catch((err) => {
        Logger.warn('[LiveAlert] change failed:', err);
        dispatch({ kind: 'failed', key, id });
      });
  };

  return { on: shownFor(view, key), busy: view.key === key && view.busy, setOn };
}
