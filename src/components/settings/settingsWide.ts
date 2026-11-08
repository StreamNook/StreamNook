// Whether the open Settings tab asks for the wide window the Stream Overlay
// studio uses. Profile > Customize asks while it shows, so a big cosmetics
// collection gets room instead of a narrow column.

import { useEffect, useSyncExternalStore } from 'react';

let wide = false;
const listeners = new Set<() => void>();

function set(next: boolean): void {
  if (wide === next) return;
  wide = next;
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

const read = () => wide;

export function useSettingsWide(): boolean {
  return useSyncExternalStore(subscribe, read, read);
}

/** Ask for the wide window while `on` and mounted. */
export function useRequestSettingsWide(on: boolean): void {
  useEffect(() => {
    if (!on) return;
    set(true);
    return () => set(false);
  }, [on]);
}
