// Tells Rust which whisper conversation this page shows
// (services/whisper_claims.rs). Each call carries this page load's id and a
// number that only grows, so Rust can drop a call that arrives after a newer
// one, and a reloaded page starts with no conversation claimed.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

function makePageLoadId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }
}

const PAGE_LOAD = makePageLoadId();
let seq = 0;

/** This page now shows `key` (null: no conversation). */
export function claimWhisper(key: string | null): void {
  seq += 1;
  invoke('whisper_set_active', { key, pageLoad: PAGE_LOAD, seq }).catch((err) =>
    Logger.debug('[Whispers] set active failed:', err),
  );
}

/** Called once as the page boots: nothing is shown yet. */
export function clearWhisperClaim(): void {
  claimWhisper(null);
}
