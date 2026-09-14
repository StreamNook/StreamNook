// Update checking for the sideloaded Android build.
//
// There is no store to do this for us. The APK is downloaded from
// streamnook.app, so nothing tells a user a newer build exists unless the app
// asks - and until now nothing did: `/api/v1/update-android` has been serving a
// manifest that no client ever read.
//
// The flow is deliberately hands-off. We do NOT download or install anything:
// we open the same public download URL in the browser, and Android's package
// installer takes over from there (the manifest ships REQUEST_INSTALL_PACKAGES
// for exactly that). Downloading an APK ourselves would mean owning storage,
// permissions, integrity checking and the installer intent, to arrive at the
// same place the browser reaches on its own.
import { Logger } from '../utils/logger';
import { openExternal } from './openExternal';

const MANIFEST_URL = 'https://streamnook.app/api/v1/update-android';
const DOWNLOAD_URL = 'https://streamnook.app/download/android';

/**
 * The version this build actually is.
 *
 * NOT the `get_current_app_version` command, which returns
 * `env!("CARGO_PKG_VERSION")` - the version compiled in from Cargo.toml, which
 * is the DESKTOP number. Android's version comes from the `tauri.android.conf.json`
 * override, and that override never reaches Cargo: it feeds the generated
 * tauri.properties and Gradle's versionName only.
 *
 * That mismatch made the first update check useless in the quietest possible
 * way. It compared latest 0.1.1 against "current" 8.3.9, concluded there was
 * nothing newer, and returned null - no error, no log, just a row that never
 * appeared.
 *
 * `getVersion()` reads the Tauri config version, and the CLI merges the
 * platform config before embedding it, so on Android it is the Android number.
 */
export async function getAppVersion(): Promise<string> {
  const { getVersion } = await import('@tauri-apps/api/app');
  return getVersion();
}

export interface AndroidUpdate {
  current: string;
  latest: string;
  notes?: string;
  /** Bytes, for showing the size before someone commits to a download. */
  size?: number;
}

/**
 * Numeric-segment comparison. Deliberately not a semver library for one
 * greater-than check, and deliberately NOT a string compare: "0.10.0" sorts
 * before "0.9.0" lexically, which would silently stop offering updates the
 * moment a minor version reaches double digits.
 */
function isNewer(latest: string, current: string): boolean {
  const a = latest.split('.').map((n) => parseInt(n, 10) || 0);
  const b = current.split('.').map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

/** Returns the update when one is available, otherwise null. Never throws. */
export async function checkForAndroidUpdate(): Promise<AndroidUpdate | null> {
  try {
    const current = await getAppVersion();
    // no-store: an edge-cached manifest is how a release goes out and nobody
    // hears about it for an hour.
    const res = await fetch(MANIFEST_URL, { cache: 'no-store' });
    // 503 is the documented "no build published yet" state, not a failure.
    if (!res.ok) return null;
    const m = (await res.json()) as { version?: string; notes?: string; size?: number };
    // Logged at warn because Logger.debug is silenced by default and a phone
    // has no devtools. This is the line that would have caught the version
    // mismatch immediately instead of costing a release.
    Logger.warn(`[Update] running ${current}, published ${m.version ?? 'none'}`);
    if (!m.version || !isNewer(m.version, current)) return null;
    return { current, latest: m.version, notes: m.notes, size: m.size };
  } catch (err) {
    // A failed check must never block the app or nag. Silence is correct here:
    // the user did not ask for this, and it retries next launch.
    Logger.warn('[Update] check failed:', err);
    return null;
  }
}

/**
 * Hands off to the browser, which hands off to Android's package installer.
 *
 * Returns whether the handoff was accepted, so a refusal can be said out loud.
 * This used to swallow the failure, and since the shell plugin's `open` cannot
 * work on Android at all (see `openExternal`), the update button did nothing
 * and said nothing, for everyone, every time.
 */
export async function openAndroidUpdate(): Promise<boolean> {
  return openExternal(DOWNLOAD_URL);
}
