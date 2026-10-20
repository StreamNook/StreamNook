// Whether the system lets StreamNook show its own notifications, as Rust reads
// it (`system_notification_permission`, services/system_notify.rs). Only macOS
// asks, so everywhere else it is granted without a call. On a Mac it is read
// when a page mounts and again each time the window comes back into focus,
// since a refusal is undone in System Settings, outside the app.
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { IS_MAC } from './platform';
import { Logger } from './logger';

/** `default` while the system has not asked yet. */
export type SystemNotifyPermission = 'granted' | 'denied' | 'default';

async function read(ask: boolean): Promise<SystemNotifyPermission> {
  try {
    return await invoke<SystemNotifyPermission>('system_notification_permission', { ask });
  } catch (err) {
    // An unreadable answer is not a refusal: saying "turned off" when it is
    // not would send someone to fix a setting that is fine.
    Logger.warn('[systemNotifyPermission] read failed:', err);
    return 'granted';
  }
}

/** The answer (null on a Mac until read), and `ask`, which shows the system's
 *  prompt when it has never been shown and returns the new answer. */
export function useSystemNotifyPermission(): {
  permission: SystemNotifyPermission | null;
  ask: () => Promise<SystemNotifyPermission>;
} {
  const [permission, setPermission] = useState<SystemNotifyPermission | null>(IS_MAC ? null : 'granted');

  useEffect(() => {
    if (!IS_MAC) return;
    let live = true;
    const refresh = () => {
      void read(false).then((p) => {
        if (live) setPermission(p);
      });
    };
    refresh();
    window.addEventListener('focus', refresh);
    return () => {
      live = false;
      window.removeEventListener('focus', refresh);
    };
  }, []);

  const ask = useCallback(async (): Promise<SystemNotifyPermission> => {
    if (!IS_MAC) return 'granted';
    const p = await read(true);
    setPermission(p);
    return p;
  }, []);

  return { permission, ask };
}

/** The system's own notification settings (macOS System Settings, Windows Settings). */
export function openSystemNotifySettings(): void {
  invoke('open_system_notification_settings').catch((err) =>
    Logger.warn('[systemNotifyPermission] open settings failed:', err),
  );
}
