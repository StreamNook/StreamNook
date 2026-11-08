// The look being hovered in Profile > Customize (an Atmosphere, a background,
// or none). Only the chat preview reads it, so moving the pointer across the
// rows repaints that one line instead of re-rendering every badge and paint
// on the page.

import { useSyncExternalStore } from 'react';

let hovered: string | null = null;
const listeners = new Set<() => void>();

export function setHoveredTheme(id: string | null): void {
  if (hovered === id) return;
  hovered = id;
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

const read = () => hovered;

export function useHoveredTheme(): string | null {
  return useSyncExternalStore(subscribe, read, read);
}
