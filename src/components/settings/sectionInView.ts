import { useSyncExternalStore } from 'react';

// The Settings section being read, as the section rail (SectionNav) sees it:
// its label (`data-settings-section`), or null. A preview beside the settings
// (Settings > Chat's chat column) follows it to show that section's pieces.
let current: string | null = null;
const listeners = new Set<() => void>();

export function setSectionInView(label: string | null): void {
  if (label === current) return;
  current = label;
  listeners.forEach((l) => l());
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

export const useSectionInView = (): string | null => useSyncExternalStore(subscribe, () => current);
