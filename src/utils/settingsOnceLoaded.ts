// The first settings load, as a promise anything booting can share.
//
// `AppStore.loadSettings` reads settings from disk once at boot (through the
// entry-module preload in bootPreload.ts). Other stores that need a slice of
// them at boot used to issue their own `load_settings`, so the same file was
// read and serialised twice before the page had painted. They await this
// instead; a later reload (another window saved) still does its own invoke,
// because by then the settings really may have changed.
//
// Its own module, with no imports, so a store can take it without pulling the
// whole AppStore graph into its unit tests.

import type { Settings } from '../types';

let settle: ((settings: Settings | null) => void) | null = null;

const first = new Promise<Settings | null>((resolve) => {
  settle = resolve;
});

/** Resolves with the first settings the app loaded, or null when that load
 *  failed; a caller then falls back to its own read. Never rejects. */
export function settingsOnceLoaded(): Promise<Settings | null> {
  return first;
}

/** Called by the first `loadSettings`, with what it read or null on failure.
 *  Later calls are no-ops: the promise is the FIRST load by definition. */
export function settleFirstSettings(settings: Settings | null): void {
  settle?.(settings);
  settle = null;
}
