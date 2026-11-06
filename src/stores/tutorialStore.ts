import { create } from 'zustand';

export interface TutorialState {
  order: string[];
  focusedId: string | null;
  isDocked: boolean;
  setOrder: (order: string[]) => void;
  setFocusedId: (id: string | null) => void;
  setIsDocked: (docked: boolean) => void;
  reset: () => void;
}

const DEFAULT_ORDER = [
  'tutorial::add',
  'tutorial::focus',
  'tutorial::chat',
  'tutorial::layout',
  'tutorial::spotlight',
  'tutorial::drag',
  'tutorial::dock',
  'tutorial::presets',
  'tutorial::sync',
];

export const useTutorialStore = create<TutorialState>((set) => ({
  order: DEFAULT_ORDER,
  focusedId: 'tutorial::focus', // Pre-focus "focus" to teach it? No, keep it null to let user do it. Actually null is fine. User asked "click it to make it glow". Let's start with null.
  isDocked: false,

  setOrder: (order) => set({ order }),
  setFocusedId: (focusedId) => set({ focusedId }),
  setIsDocked: (isDocked) => set({ isDocked }),
  
  reset: () =>
    set({
      order: DEFAULT_ORDER,
      focusedId: null,
      isDocked: false,
    }),
}));
