// The alert types this build sends, as Rust lists them (services/alert_types.rs,
// `get_alert_types`): the Notifications page's "Notify me about" rows and the
// setup wizard's Notifications step both draw from it, in its order and with
// its words, so desktop and the two phones list the same thing. A type this
// build cannot send is simply not in the list. Desktop's Settings and setup
// read it from here too.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';
import type { LiveNotificationSettings } from '../types';

/** Mirrors Rust `alert_types::AlertType`. */
export interface AlertType {
  /** `live`, `mentions`, `whispers`, `drops`, `badges`, and on desktop `updates` and the rest. */
  id: string;
  title: string;
  detail: string;
  /** The dotted settings path the switch writes (`live_notifications.show_live_notifications`). */
  setting: string;
  /** The Android notification channel it posts under. */
  channel: string;
  /** Offered in the setup wizard's Notifications step. */
  setup: boolean;
}

let cached: AlertType[] | null = null;
let pending: Promise<AlertType[]> | null = null;

/** The list, read once per run (it is fixed for a build). An empty list when the read fails. */
export function loadAlertTypes(): Promise<AlertType[]> {
  if (cached) return Promise.resolve(cached);
  pending ??= invoke<AlertType[]>('get_alert_types')
    .then((list) => {
      cached = list;
      return list;
    })
    .catch((err) => {
      Logger.warn('[alertTypes] get_alert_types failed:', err);
      pending = null;
      return [];
    });
  return pending;
}

/** The list once read; null until then. */
export function useAlertTypes(): AlertType[] | null {
  const [list, setList] = useState<AlertType[] | null>(cached);
  useEffect(() => {
    if (cached) return;
    let live = true;
    void loadAlertTypes().then((l) => {
      if (live) setList(l);
    });
    return () => {
      live = false;
    };
  }, []);
  return list;
}

/** The `live_notifications` key a type's switch writes, or null for a path outside that group. */
export function alertKey(type: AlertType): keyof LiveNotificationSettings | null {
  const [group, key] = type.setting.split('.');
  return group === 'live_notifications' && key ? (key as keyof LiveNotificationSettings) : null;
}

/** Whether a type is switched on in these settings (every switch defaults on). */
export function alertOn(type: AlertType, prefs: Partial<LiveNotificationSettings> | undefined): boolean {
  const key = alertKey(type);
  return key ? prefs?.[key] !== false : false;
}
