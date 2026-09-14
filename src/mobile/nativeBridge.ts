// Thin wrappers over the MainActivity JavascriptInterface (SNInsets). Every
// call is best-effort: the bridge is absent on desktop and in dev browsers.
interface SNBridge {
  get?(): string;
  setImmersive?(immersive: boolean): void;
  setBarsLightContent?(light: boolean): void;
  setKeepScreenOn?(on: boolean): void;
  setPipEligible?(eligible: boolean): void;
  enterPip?(): void;
  isInPip?(): boolean;
  consumePipClosed?(): boolean;
  setPipSourceRect?(l: number, t: number, r: number, b: number): void;
  setPipMuted?(muted: boolean): void;
  share?(text: string, subject: string): void;
  areNotificationsEnabled?(): boolean;
  channelImportance?(id: string): number;
  shouldShowNotificationRationale?(): boolean;
  openNotificationSettings?(): void;
  openChannelSettings?(id: string): void;
  isIgnoringBatteryOptimizations?(): boolean;
  requestIgnoreBatteryOptimizations?(): void;
  scheduleBackgroundChecks?(intervalMinutes: number): void;
  cancelBackgroundChecks?(): void;
  runNotifyCheckNow?(): void;
  consumePendingChannel?(): string;
  mediaSessionStart?(title: string, artist: string, artUrl: string, playing: boolean): void;
  mediaSessionStop?(): void;
  displayShortEdge?(): number;
}

function bridge(): SNBridge | undefined {
  return (window as Window & { SNInsets?: SNBridge }).SNInsets;
}

export function setImmersive(immersive: boolean): void {
  try {
    bridge()?.setImmersive?.(immersive);
  } catch {
    /* bridge absent */
  }
}

/** Icon colour for the system bars. `light` means light CONTENT (a white clock),
 *  which is what a dark theme background needs. */
export function setBarsLightContent(light: boolean): void {
  try {
    bridge()?.setBarsLightContent?.(light);
  } catch {
    /* bridge absent */
  }
}

export function setKeepScreenOn(on: boolean): void {
  try {
    bridge()?.setKeepScreenOn?.(on);
  } catch {
    /* bridge absent */
  }
}

export function setPipEligible(eligible: boolean): void {
  try {
    bridge()?.setPipEligible?.(eligible);
  } catch {
    /* bridge absent */
  }
}

export function enterPip(): void {
  try {
    bridge()?.enterPip?.();
  } catch {
    /* bridge absent */
  }
}

/**
 * Whether the activity is in the system PiP window, read SYNCHRONOUSLY.
 *
 * The `sn:pip` event and the `dataset.snPip` mirror both arrive via an async
 * `evaluateJavascript` from onPictureInPictureModeChanged, which races the
 * WebView's own `visibilitychange`. Anything that has to tell real backgrounding
 * from PiP must not depend on winning that race.
 *
 * Returns null when there is no bridge, so callers can tell "unknown" from
 * "not pipped".
 */
export function isInPip(): boolean | null {
  try {
    const b = bridge();
    if (typeof b?.isInPip === 'function') return b.isInPip();
  } catch {
    /* bridge absent */
  }
  return null;
}

/**
 * True once if the PiP window was CLOSED rather than expanded back into the app.
 *
 * Reading it clears it on the native side. Closing PiP does not finish the
 * activity, so without this the stream stays loaded and comes back the next time
 * the app is opened, which is not what dismissing the window means.
 */
export function consumePipClosed(): boolean {
  try {
    return bridge()?.consumePipClosed?.() ?? false;
  } catch {
    return false;
  }
}

/** Video rect in DEVICE pixels, so the OS animates PiP from the video rather
 *  than cropping and scaling the whole activity. */
export function setPipSourceRect(l: number, t: number, r: number, b: number): void {
  try {
    bridge()?.setPipSourceRect?.(l, t, r, b);
  } catch {
    /* bridge absent */
  }
}

/** Keeps the PiP window's mute RemoteAction showing the right icon. */
export function setPipMuted(muted: boolean): void {
  try {
    bridge()?.setPipMuted?.(muted);
  } catch {
    /* bridge absent */
  }
}

/** Opens the system share sheet. Falls back to the clipboard if unavailable. */
export function shareText(text: string, subject = ''): boolean {
  try {
    const b = bridge();
    if (b?.share) {
      b.share(text, subject);
      return true;
    }
  } catch {
    /* fall through */
  }
  void navigator.clipboard?.writeText(text).catch(() => {});
  return false;
}

// ---- Notifications --------------------------------------------------------
//
// The notification plugin only knows about the runtime permission. Two other
// states silence notifications just as completely and it cannot see either: the
// app-level switch in system settings, and a single category set to
// IMPORTANCE_NONE. Both are read here so the panel can tell someone what is
// actually wrong instead of reporting that everything is fine.
//
// Each returns null when the bridge is absent, so "unknown" stays distinct from
// "off" and the desktop/dev-browser case renders nothing rather than a scare.

export function areNotificationsEnabled(): boolean | null {
  try {
    const b = bridge();
    if (typeof b?.areNotificationsEnabled === 'function') return b.areNotificationsEnabled();
  } catch {
    /* bridge absent */
  }
  return null;
}

/** OS importance for one channel: 0 is blocked, -1 not created yet, null unknown. */
export function channelImportance(id: string): number | null {
  try {
    const b = bridge();
    if (typeof b?.channelImportance === 'function') return b.channelImportance(id);
  } catch {
    /* bridge absent */
  }
  return null;
}

/**
 * Whether Android would still show a rationale for the notification permission.
 *
 * False plus "not granted" means the user has permanently declined, and every
 * further in-app request returns denied without showing anything. That is the
 * one state where the only honest UI is a link into system settings.
 */
export function shouldShowNotificationRationale(): boolean | null {
  try {
    const b = bridge();
    if (typeof b?.shouldShowNotificationRationale === 'function') {
      return b.shouldShowNotificationRationale();
    }
  } catch {
    /* bridge absent */
  }
  return null;
}

export function openNotificationSettings(): void {
  try {
    bridge()?.openNotificationSettings?.();
  } catch {
    /* bridge absent */
  }
}

export function openChannelSettings(id: string): void {
  try {
    bridge()?.openChannelSettings?.(id);
  } catch {
    /* bridge absent */
  }
}

// ---- Background delivery --------------------------------------------------

/**
 * Whether the app is exempt from battery optimisation.
 *
 * This decides whether background notifications work at all, not just how
 * promptly they arrive: Android withholds network from jobs entirely in the
 * Rare and Restricted standby buckets, and the exemption is what lifts an app
 * out of them.
 */
export function isIgnoringBatteryOptimizations(): boolean | null {
  try {
    const b = bridge();
    if (typeof b?.isIgnoringBatteryOptimizations === 'function') {
      return b.isIgnoringBatteryOptimizations();
    }
  } catch {
    /* bridge absent */
  }
  return null;
}

export function requestIgnoreBatteryOptimizations(): void {
  try {
    bridge()?.requestIgnoreBatteryOptimizations?.();
  } catch {
    /* bridge absent */
  }
}

export function scheduleBackgroundChecks(intervalMinutes: number): void {
  try {
    bridge()?.scheduleBackgroundChecks?.(intervalMinutes);
  } catch {
    /* bridge absent */
  }
}

export function cancelBackgroundChecks(): void {
  try {
    bridge()?.cancelBackgroundChecks?.();
  } catch {
    /* bridge absent */
  }
}

/** One immediate notification poll; the periodic slot may be minutes away. */
export function runNotifyCheckNow(): void {
  try {
    bridge()?.runNotifyCheckNow?.();
  } catch {
    /* bridge absent */
  }
}

/**
 * Channel login from a tapped notification, or null.
 *
 * Reading it clears it natively, same contract as consumePipClosed. Drained on
 * mount because a tap that cold-starts the app has no WebView to be pushed to;
 * the warm case arrives through `window.__SN_OPEN_CHANNEL__` instead.
 */
export function consumePendingChannel(): string | null {
  try {
    const channel = bridge()?.consumePendingChannel?.();
    return channel && channel.length > 0 ? channel : null;
  } catch {
    return null;
  }
}

/**
 * Start or update the lock-screen media session.
 *
 * Also the thing that KEEPS AUDIO ALIVE with the screen off: the foreground
 * service it starts is what stops Android freezing the process, and its running
 * state is what tells MainActivity to undo wry's `WebView.onPause()`.
 *
 * Call on every play/pause too - the web layer is the source of truth for the
 * play state, and the notification button only sends a command.
 */
export function mediaSessionStart(
  title: string,
  artist: string,
  artUrl: string,
  playing: boolean,
): void {
  try {
    bridge()?.mediaSessionStart?.(title, artist, artUrl, playing);
  } catch {
    /* bridge absent */
  }
}

/** Tear the session down. Audio no longer survives the screen going off. */
export function mediaSessionStop(): void {
  try {
    bridge()?.mediaSessionStop?.();
  } catch {
    /* bridge absent */
  }
}
