import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

/**
 * What this client IS, as Rust sees it. The one source for any version or
 * platform we report to the backend.
 *
 * Mirrors `ClientIdentity` in `src-tauri/src/services/client_identity.rs`; the
 * field names are the wire format shared with `streamnook.app`, so renaming one
 * is a cross-repo change.
 */
export interface ClientIdentity {
  /** Merged-config version: 8.x on desktop, 0.1.x on Android. */
  app_version: string;
  /** `windows` | `macos` | `linux` | `android` | `ios`. */
  platform: string;
  /** `x86_64` | `aarch64`. */
  arch: string;
  /** `<platform>-<arch>`, the update manifest's own key. */
  target: string;
  /** `release` | `debug`. */
  channel: string;
}

let cached: Promise<ClientIdentity> | null = null;
let resolved: ClientIdentity | null = null;

/**
 * The running client's identity. Compile-time constants on the Rust side, so
 * this is asked once per window and memoized; a failure clears the cache so the
 * next caller retries rather than being stuck with a rejected promise.
 */
export function getClientIdentity(): Promise<ClientIdentity> {
  cached ??= invoke<ClientIdentity>('get_client_identity')
    .then((id) => {
      resolved = id;
      return id;
    })
    .catch((e) => {
      cached = null;
      throw e;
    });
  return cached;
}

/**
 * The identity if it has already been fetched, else null.
 *
 * For the sync paths that cannot await (the presence payload is built inside a
 * re-key). Callers must have a correct answer for null; `primeClientIdentity`
 * at boot is what makes that window short.
 */
export function getClientIdentitySync(): ClientIdentity | null {
  return resolved;
}

/** Warm the cache at startup so the first reporter does not pay the round trip. */
export function primeClientIdentity(): void {
  void getClientIdentity().catch((e) => {
    Logger.warn('[ClientIdentity] could not read client identity:', e);
  });
}

/**
 * The app version.
 *
 * Deliberately NOT the `get_current_app_version` / `get_app_version` commands:
 * both return `env!("CARGO_PKG_VERSION")`, which is the DESKTOP number even
 * inside an Android build, because the `tauri.android.conf.json` version
 * override feeds Gradle and never reaches Cargo. Android reported 8.3.9 to
 * Supabase for exactly that reason. This reads the merged config, so it is
 * right on both platforms.
 */
export function getAppVersion(): Promise<string> {
  return getClientIdentity().then((id) => id.app_version);
}
