import { invoke } from '@tauri-apps/api/core';

let cached: Promise<string> | null = null;

/** The app version is a compile-time constant; ask Rust once per session. */
export function getAppVersion(): Promise<string> {
  cached ??= invoke<string>('get_current_app_version').catch((e) => {
    cached = null;
    throw e;
  });
  return cached;
}
