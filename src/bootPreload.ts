import { invoke } from '@tauri-apps/api/core';

// Fires the settings IPC as soon as the entry module evaluates, so the disk
// read overlaps the lazy route chunks loading instead of running after them.
// Popout windows load their own state and skip the preload entirely.
let pending: Promise<unknown | null> | null = null;

const hash = window.location.hash;
const isPopout =
  hash.startsWith('#/profile') || hash.startsWith('#/multichat') || hash.startsWith('#/plugin/');
if (!isPopout) {
  pending = invoke('load_settings').catch(() => null);
}

// Consume-once: the first loadSettings takes the preload; every later call
// (e.g. cross-window settings sync) gets null and does a fresh invoke.
export function takePreloadedSettings(): Promise<unknown | null> | null {
  const p = pending;
  pending = null;
  return p;
}
