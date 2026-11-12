// Which optional settings rows this window can offer right now.
//
// A row that exists only for some accounts (it is drawn once Rust has the data
// behind it) is named here by a need. The settings search drops an entry whose
// need is not met, so a result never leads to a row that is not on the page.
// The version counter lets a memo that holds search results run again when a
// need flips.
import { useSyncExternalStore } from 'react';

const met = new Map<string, boolean>();
const listeners = new Set<() => void>();
let version = 0;

/** Records whether a need is met. A repeat of the current answer changes nothing. */
export function setSettingNeed(name: string, isMet: boolean): void {
  if ((met.get(name) ?? false) === isMet) return;
  met.set(name, isMet);
  version += 1;
  for (const listener of Array.from(listeners)) listener();
}

/** Whether a need is met. A need nobody has reported is not met. */
export function settingNeedMet(name: string): boolean {
  return met.get(name) === true;
}

/** Moves every time any need flips. */
export function settingNeedsVersion(): number {
  return version;
}

export function subscribeSettingNeeds(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Re-renders the caller when a need flips; the value is for memo dependencies. */
export function useSettingNeedsVersion(): number {
  return useSyncExternalStore(subscribeSettingNeeds, settingNeedsVersion, settingNeedsVersion);
}
