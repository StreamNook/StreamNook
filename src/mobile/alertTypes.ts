// The alert types this build sends, as Rust lists them (services/alert_types.rs,
// `get_alert_types`): the Notifications page's "Notify me about" rows and the
// setup wizard's Notifications step both draw from it, in its order and with
// its words, so desktop and the two phones list the same thing. A type this
// build cannot send is simply not in the list. Desktop's Settings and setup
// read it from here too.
import { useEffect, useState } from 'react';
import { invokeList } from '../utils/invokeList';
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

/** The list, read once per run (it is fixed for a build). Retried while Rust
 *  does not answer (`invokeList`); an empty list only when every try failed,
 *  and the next caller then tries again. */
export function loadAlertTypes(): Promise<AlertType[]> {
  if (cached) return Promise.resolve(cached);
  pending ??= invokeList<AlertType>('get_alert_types').then((list) => {
    if (list.length > 0) cached = list;
    else pending = null;
    return list;
  });
  return pending;
}

/** The list once read; null until then. */
export function useAlertTypes(): AlertType[] | null {
  const [list, setList] = useState<AlertType[] | null>(cached);
  useEffect(() => {
    // Always asks, even when the list is already in: it can land after this
    // hook's first render and before this effect, and an early return here
    // left the page on null for the whole run (Notify me about empty).
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
