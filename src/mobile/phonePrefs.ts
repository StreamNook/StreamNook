// Phone-only preferences that shape the SHELL rather than the app: which tab
// the app opens on, whether a mention buzzes. They live beside the watch
// layout and split (see watch/watchLayout.ts) in localStorage, not in the
// shared Settings model: nothing in Rust or on desktop reads them, and a
// settings-file field for a phone shell choice would be one more thing for
// the desktop struct to carry for nobody.
//
// Held in a store rather than read ad hoc so the settings row and the code
// that acts on the choice always agree within the same session.
import { create } from 'zustand';
import type { MobileTab } from './navStore';

const START_TAB_KEY = 'sn-start-tab';
const MENTION_HAPTIC_KEY = 'sn-mention-haptic';

const TABS: readonly MobileTab[] = ['following', 'browse', 'rewards', 'you'];

function readStartTab(): MobileTab {
  try {
    const v = localStorage.getItem(START_TAB_KEY);
    return (TABS as readonly string[]).includes(v ?? '') ? (v as MobileTab) : 'following';
  } catch {
    return 'following';
  }
}

function readMentionHaptic(): boolean {
  try {
    return localStorage.getItem(MENTION_HAPTIC_KEY) !== 'off';
  } catch {
    return true;
  }
}

interface PhonePrefs {
  /** The tab the app opens on, and the one the back gesture unwinds to. */
  startTab: MobileTab;
  /** A short vibration when a message mentions you. */
  mentionHaptic: boolean;
  setStartTab: (tab: MobileTab) => void;
  setMentionHaptic: (on: boolean) => void;
}

export const usePhonePrefs = create<PhonePrefs>((set) => ({
  startTab: readStartTab(),
  mentionHaptic: readMentionHaptic(),
  setStartTab: (tab) => {
    try {
      localStorage.setItem(START_TAB_KEY, tab);
    } catch {
      /* private mode; the choice lasts the session */
    }
    set({ startTab: tab });
  },
  setMentionHaptic: (on) => {
    try {
      localStorage.setItem(MENTION_HAPTIC_KEY, on ? 'on' : 'off');
    } catch {
      /* private mode; the choice lasts the session */
    }
    set({ mentionHaptic: on });
  },
}));

/** Synchronous read for code that runs before React (the nav store's seed). */
export const startTab = (): MobileTab => usePhonePrefs.getState().startTab;
