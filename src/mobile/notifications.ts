// Native system notifications for the mobile shell.
//
// The plugin's JS wrapper is unusable here: `sendNotification` calls
// `new window.Notification(...)`, and Android System WebView has no Web
// Notifications API, so it throws. The Rust commands are invoked directly
// instead (the ACL exposes allow-notify / allow-request-permission /
// allow-is-permission-granted).
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';
import {
  areNotificationsEnabled,
  cancelBackgroundChecks,
  channelImportance,
  scheduleBackgroundChecks,
  shouldShowNotificationRationale,
} from './nativeBridge';
import type { LiveNotificationSettings } from '../types';

export async function isNotificationPermissionGranted(): Promise<boolean> {
  try {
    return await invoke<boolean>('plugin:notification|is_permission_granted');
  } catch (err) {
    Logger.warn('[Notifications] permission probe failed:', err);
    return false;
  }
}

/**
 * `default` has never been asked, `denied` can still be asked again, `blocked`
 * cannot: Android stops showing the dialog after the second refusal and every
 * later request returns denied without displaying anything.
 */
export type NotifyPermission = 'granted' | 'default' | 'denied' | 'blocked';

// Android reports shouldShowRequestPermissionRationale as false BOTH before the
// first ask and after a permanent refusal, so it cannot separate them alone.
// This flag supplies the missing bit. It is deliberately not load-bearing: the
// panel offers the system-settings route whenever permission is missing, so a
// cleared flag costs a slightly wrong sentence, never a dead end.
const ASKED_KEY = 'sn-notif-asked';

function markAsked(): void {
  try {
    localStorage.setItem(ASKED_KEY, '1');
  } catch {
    /* private mode / storage disabled */
  }
}

function hasAsked(): boolean {
  try {
    return localStorage.getItem(ASKED_KEY) === '1';
  } catch {
    return false;
  }
}

export async function getNotificationPermission(): Promise<NotifyPermission> {
  if (await isNotificationPermissionGranted()) return 'granted';
  const rationale = shouldShowNotificationRationale();
  if (rationale === true) return 'denied';
  return hasAsked() ? 'blocked' : 'default';
}

/** Returns true when permission is granted (asking only if not already). */
export async function ensureNotificationPermission(): Promise<boolean> {
  if (await isNotificationPermissionGranted()) return true;
  markAsked();
  try {
    const result = await invoke<string>('plugin:notification|request_permission');
    return result === 'granted';
  } catch (err) {
    // Surface it: mapping every failure to "denied" makes a broken plugin
    // registration indistinguishable from the user declining.
    Logger.error('[Notifications] permission request failed:', err);
    return false;
  }
}

/**
 * Everything that can silence notifications, in one read.
 *
 * The plugin only knows about the runtime permission. Someone can grant it and
 * still receive nothing because the app is switched off in system settings, or
 * receive some categories and not others because one channel is at
 * IMPORTANCE_NONE. Reporting only the permission is how a panel ends up telling
 * someone everything is fine while they get nothing.
 */
export interface NotifyDelivery {
  permission: NotifyPermission;
  /** App-level switch in system settings. null when the bridge is absent. */
  appEnabled: boolean | null;
  /** Channel ids the user has silenced in Android's own settings. */
  blockedChannels: NotifyChannelId[];
}

export async function getNotifyDelivery(): Promise<NotifyDelivery> {
  const permission = await getNotificationPermission();
  const blocked = (Object.values(NOTIFY_CHANNEL) as NotifyChannelId[]).filter(
    // 0 is IMPORTANCE_NONE. -1 means the channel does not exist yet, which is
    // not a block, and null means no bridge to ask.
    (id) => channelImportance(id) === 0,
  );
  return { permission, appEnabled: areNotificationsEnabled(), blockedChannels: blocked };
}

/**
 * Android notification channels, one per category.
 *
 * Without these, every notification lands in one bucket and Android's own
 * per-app settings offer a single on/off for the whole app. With them the OS
 * lists the categories separately, so someone can silence Drops while keeping
 * live alerts, or give each its own importance and sound. That system-level
 * control is the part an in-app toggle cannot provide.
 *
 * Channels are IMMUTABLE once created: Android ignores later changes to an
 * existing channel's name or importance, so changing either means minting a new
 * id. Treat the values below as permanent.
 *
 * `notification:default` already grants `allow-create-channel`, so this needs
 * no capability change.
 */
export const NOTIFY_CHANNEL = {
  live: 'live-channels',
  drops: 'drops',
  badges: 'badges',
  points: 'channel-points',
} as const;

export type NotifyChannelId = (typeof NOTIFY_CHANNEL)[keyof typeof NOTIFY_CHANNEL];

// Importance: 4 = High (heads-up), 3 = Default (shade + sound), 2 = Low (silent).
const CHANNELS: { id: string; name: string; description: string; importance: number }[] = [
  {
    id: NOTIFY_CHANNEL.live,
    name: 'Channels going live',
    description: 'A channel you follow started streaming',
    importance: 4,
  },
  {
    id: NOTIFY_CHANNEL.drops,
    name: 'Drops',
    description: 'Rewards ready to claim, and rewards claimed for you',
    importance: 3,
  },
  {
    id: NOTIFY_CHANNEL.badges,
    name: 'New badges',
    description: 'A new global badge became available',
    importance: 3,
  },
  {
    id: NOTIFY_CHANNEL.points,
    name: 'Channel points',
    description: 'Bonus chests collected in the background',
    // Quiet on purpose: this can fire every quarter hour while watching, so it
    // belongs in the shade rather than on top of whatever you are doing.
    importance: 2,
  },
];

/** Idempotent: Android no-ops a channel that already exists. */
export async function ensureNotificationChannels(): Promise<void> {
  for (const channel of CHANNELS) {
    try {
      await invoke('plugin:notification|create_channel', channel);
    } catch (err) {
      // Not fatal. A notification naming an unknown channel still posts to the
      // default one, so the worst case is losing the per-category control.
      Logger.warn(`[Notifications] channel "${channel.id}" failed:`, err);
    }
  }
}

/**
 * Android status-bar icon, as a drawable resource name.
 *
 * Set on every notification. Without it the plugin falls back to the app icon,
 * and Android renders a small icon using its alpha only, so full-colour launcher
 * art collapses into a white square.
 */
const SMALL_ICON = 'ic_stat_notify';

export interface SystemNotification {
  title: string;
  body?: string;
  largeBody?: string;
  /**
   * Android DRAWABLE RESOURCE NAME, not a URL. Defaults to the StreamNook mark.
   *
   * Worth stating because it is not what the field name suggests and the
   * failure is silent: the plugin resolves both `icon` and `largeIcon` with
   * `Resources.getIdentifier(name, "drawable", pkg)`, so an https avatar
   * resolves to id 0 and is dropped. There is no way to show a remote image
   * through this plugin at all; that needs a NotificationCompat call that
   * fetches the bitmap itself, which is what the background worker does.
   */
  icon?: string;
  /** Which OS category this belongs to. Omit only for one-off diagnostics. */
  channelId?: NotifyChannelId;
  /**
   * Stable notification id. Omitted, the plugin picks a RANDOM id, so a repeat
   * of the same news stacks a duplicate row; with a stable id it replaces.
   * Derive it with `stableNotifyId` from the same key the background worker
   * hashes for its ids.
   */
  id?: number;
}

/**
 * Java `String.hashCode` over UTF-16 units. Matching Java is the point: the
 * background worker ids its notifications with `channelId.hashCode()` in
 * Kotlin, so hashing the same key here lands both lanes on the same id and a
 * cross-lane repeat degrades to a replace instead of a duplicate.
 */
export function stableNotifyId(key: string): number {
  let h = 0;
  for (let i = 0; i < key.length; i++) {
    h = (Math.imul(h, 31) + key.charCodeAt(i)) | 0;
  }
  return h;
}

/**
 * Brings the background poll in line with the settings, in one place.
 *
 * Called at boot and whenever either switch changes, so the scheduled work can
 * never outlive the preference that asked for it. Scheduling is idempotent, so
 * calling this more often than necessary is free.
 */
export function syncBackgroundChecks(prefs: Partial<LiveNotificationSettings> | undefined): void {
  // The worker is the single delivery lane, so the master switch is the only
  // thing that turns it off. (`background_checks` is retired: with one lane,
  // "notifications on but background checks off" would silently mean no
  // notifications at all while the UI implied otherwise.)
  if (prefs?.enabled !== false) {
    scheduleBackgroundChecks(prefs?.background_interval_minutes ?? 15);
  } else {
    cancelBackgroundChecks();
  }
}

export async function postSystemNotification(n: SystemNotification): Promise<void> {
  try {
    await invoke('plugin:notification|notify', {
      options: {
        id: n.id,
        title: n.title,
        body: n.body,
        largeBody: n.largeBody,
        icon: n.icon ?? SMALL_ICON,
        channelId: n.channelId,
      },
    });
  } catch (err) {
    Logger.warn('[Notifications] notify failed:', err);
  }
}
