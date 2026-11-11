// Which platforms the All view searches and suggests live streams from.
//
// Rust owns the choice (services/suggest_platforms.rs): it is kept under one
// settings key, written only through `set_suggest_platforms`, and read by the
// Discover lists in Rust. The page reads it here to draw the search bar's
// platform picker and to search only the platforms that are on.

import { invoke } from '@tauri-apps/api/core';
import { useMemo } from 'react';
import { useAppStore } from '../stores/AppStore';
import { WATCHABLE_PROVIDERS, type ProviderId } from '../types/providers';
import { Logger } from './logger';

/** The settings key Rust keeps the platforms left out under. */
export const SUGGEST_OFF_KEY = 'suggest_platforms_off';

/** The platforms left out, as saved: known ones only, never all of them. */
export function readSuggestOff(raw: unknown): ProviderId[] {
  if (!Array.isArray(raw)) return [];
  const off = WATCHABLE_PROVIDERS.filter((p) => raw.includes(p));
  return off.length >= WATCHABLE_PROVIDERS.length ? [] : off;
}

/** The platforms that stay on, in the app's platform order. */
export function suggestOn(off: readonly ProviderId[]): ProviderId[] {
  return WATCHABLE_PROVIDERS.filter((p) => !off.includes(p));
}

/** The next list after a press on `provider`; unchanged when that would turn
 *  the last platform off. */
export function toggledOff(off: readonly ProviderId[], provider: ProviderId): ProviderId[] {
  if (off.includes(provider)) return off.filter((p) => p !== provider);
  if (suggestOn(off).length <= 1) return [...off];
  return WATCHABLE_PROVIDERS.filter((p) => p === provider || off.includes(p));
}

function settingsValue(settings: unknown): unknown {
  return settings && typeof settings === 'object'
    ? (settings as Record<string, unknown>)[SUGGEST_OFF_KEY]
    : undefined;
}

/** The platforms left out right now, outside React (a search started in the
 *  same tick as a press already sees the press). */
export function currentSuggestOff(): ProviderId[] {
  return readSuggestOff(settingsValue(useAppStore.getState().settings));
}

/** The platforms left out, kept current as the choice changes. */
export function useSuggestOff(): ProviderId[] {
  // The raw value, so the selector hands back the same reference until the
  // setting itself changes.
  const raw = useAppStore((s) => settingsValue(s.settings));
  return useMemo(() => readSuggestOff(raw), [raw]);
}

/** Save `off`. The store takes it at once so the picker answers the press,
 *  then the stored list (Rust's own reading) once the write lands. */
export async function setSuggestOff(off: ProviderId[]): Promise<ProviderId[]> {
  const put = (value: ProviderId[]) =>
    useAppStore.setState((s) => ({ settings: { ...s.settings, [SUGGEST_OFF_KEY]: value } }));
  const before = currentSuggestOff();
  put(off);
  try {
    const stored = readSuggestOff(await invoke<string[]>('set_suggest_platforms', { off }));
    put(stored);
    return stored;
  } catch (e) {
    Logger.warn('[SuggestPlatforms] could not save the choice:', e);
    put(before);
    return before;
  }
}
