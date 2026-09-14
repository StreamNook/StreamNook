import { create } from 'zustand';
import React from 'react';

export interface TooltipState {
  isVisible: boolean;
  content: React.ReactNode | string | null;
  rect: DOMRect | null;
  side: 'top' | 'bottom' | 'left' | 'right';
  // Optional className override for the tooltip container. When set, replaces
  // the default chrome (rounded-md / bg-black/80 / border) entirely. Used by
  // StreamNookBadge to render the badge popover as a per-tier pill.
  containerClassName: string | null;
  showTooltip: (id: string, content: React.ReactNode | string, rect: DOMRect, side?: 'top' | 'bottom' | 'left' | 'right', containerClassName?: string) => void;
  hideTooltip: (id?: string) => void;
  /** Run `fire` after `delayMs` unless cancelled; one pending timer per id. */
  scheduleShow: (id: string, delayMs: number, fire: () => void) => void;
  cancelShow: (id: string) => void;
  // A unique ID or ref to track which element triggered the tooltip
  triggerId: string | null;
}

// Pending hover-delay timers, one per trigger id. Kept here (module scope,
// outside React state) so a Tooltip component needs no ref of its own: the
// timer is manager state, not render state.
const pendingShows = new Map<string, ReturnType<typeof setTimeout>>();

export const useTooltipStore = create<TooltipState>((set) => ({
  isVisible: false,
  content: null,
  rect: null,
  side: 'top',
  containerClassName: null,
  triggerId: null,
  showTooltip: (id, content, rect, side = 'top', containerClassName) => set({
    isVisible: true,
    content,
    rect,
    side,
    containerClassName: containerClassName ?? null,
    triggerId: id
  }),
  scheduleShow: (id, delayMs, fire) => {
    const prev = pendingShows.get(id);
    if (prev) clearTimeout(prev);
    pendingShows.set(id, setTimeout(() => {
      pendingShows.delete(id);
      fire();
    }, delayMs));
  },
  cancelShow: (id) => {
    const prev = pendingShows.get(id);
    if (prev) {
      clearTimeout(prev);
      pendingShows.delete(id);
    }
  },
  hideTooltip: (id?: string) => set((state) => {
    // If an ID is provided, only hide if it matches the currently active triggerId.
    if (id && state.triggerId !== id) {
      return state;
    }
    return { isVisible: false };
    // We intentionally don't clear content/rect immediately to allow the exit animation to render properly
  }),
}));
