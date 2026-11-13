// The automations status every window reads: the settings page, the chat
// header dot (main, MultiNook, the dock, MultiChat panes) and the phones' room
// tab and channel line. Rust announces each change to every window; this keeps
// the last one in a store, and reads it fresh when a window mounts.
//
// Kept outside the React Compiler paths on purpose: `useNowMs` reads the clock.

import { useEffect, useSyncExternalStore } from 'react';
import { create } from 'zustand';
import { listen } from '@tauri-apps/api/event';
import {
  AUTOMATION_STATUS_EVENT,
  automationsApi,
  dotStateFor,
  type AutomationDotState,
  type AutomationStatus,
} from '../utils/automations';
import { Logger } from '../utils/logger';

interface AutomationStatusState {
  status: AutomationStatus | null;
}

export const useAutomationStatusStore = create<AutomationStatusState>(() => ({ status: null }));

/** Bumped by every announced status, so a read that was asked for before one
 *  arrived never puts the older answer back. */
let announced = 0;

/** Read the status now (a page that just opened, or after a change it made). */
export async function refreshAutomationStatus(): Promise<AutomationStatus | null> {
  const asked = announced;
  try {
    const status = await automationsApi.status();
    if (asked === announced) useAutomationStatusStore.setState({ status });
    return status;
  } catch (err) {
    Logger.warn('[Automations] status read failed:', err);
    return null;
  }
}

/** Keeps this window's status current. Returns the stop. */
export function startAutomationStatus(): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  void listen<AutomationStatus>(AUTOMATION_STATUS_EVENT, (e) => {
    announced += 1;
    useAutomationStatusStore.setState({ status: e.payload });
  })
    .then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  void refreshAutomationStatus();
  return () => {
    stopped = true;
    unlisten?.();
  };
}

/** Mount once per window, in main, MultiChat and the phone app. */
export function useAutomationStatus(): void {
  useEffect(() => startAutomationStatus(), []);
}

/** The dot for a chat showing these rooms (composite or bare keys). */
export function useAutomationDot(keys: readonly string[]): AutomationDotState | null {
  const joined = keys.join('\n');
  return useAutomationStatusStore((s) => dotStateFor(s.status, joined ? joined.split('\n') : []));
}

let nowMs = Date.now();
const nowListeners = new Set<() => void>();
let nowTimer: ReturnType<typeof setInterval> | undefined;

function subscribeNow(listener: () => void): () => void {
  nowListeners.add(listener);
  nowMs = Date.now();
  nowTimer ??= setInterval(() => {
    nowMs = Date.now();
    nowListeners.forEach((l) => l());
  }, 30_000);
  return () => {
    nowListeners.delete(listener);
    if (nowListeners.size === 0 && nowTimer !== undefined) {
      clearInterval(nowTimer);
      nowTimer = undefined;
    }
  };
}

/** The time, moving every 30 seconds while anything reads it: for "3 min ago"
 *  words in pages that may not read the clock while they draw. */
export function useNowMs(): number {
  return useSyncExternalStore(subscribeNow, () => nowMs);
}
