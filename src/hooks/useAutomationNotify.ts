// Plays a Notify action's in-app part (its toast and its sound) in exactly one
// window. Rust sends each one to every window that could play it (main,
// MultiChat and overlay windows), and the first to claim it plays it, so it is
// heard once whichever windows are open. When none claims in time, Rust shows
// the system notification instead. A window makes no offer: Rust decides who
// can play from the window labels.
//
// Mount once per window, in main, MultiChat and the phone app.

import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useAppStore } from '../stores/AppStore';
import { playSound } from '../utils/notificationSound';
import {
  AUTOMATION_NOTIFY_EVENT,
  AUTOMATION_SPEAK_EVENT,
  type AutomationNotice,
  type AutomationSpeak,
} from '../utils/automations';
import { speakText, speechAvailable } from '../utils/speech';

function play(notice: AutomationNotice): void {
  if (notice.sound) playSound(notice.sound, Math.round(notice.volume * 100));
  if (!notice.toast) return;
  const title = notice.title.trim();
  const body = notice.body.trim();
  const words = body || title;
  if (!words) return;
  useAppStore.getState().addToast(words, 'info', undefined, body && title ? { title } : undefined);
}

/** Speaks a Speak action's line. When this device cannot speak (no voice,
 *  the Android plugin answered `{ spoken: false }`), the text is shown instead. */
async function speak(line: AutomationSpeak): Promise<void> {
  if (await speakText(line.text)) return;
  useAppStore.getState().addToast(line.text, 'info');
}

/** Offers this window as a player and plays what it wins. Returns the stop. */
export function startAutomationNotify(): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  void Promise.resolve()
    .then(() =>
      // Window-scoped: a plain `listen` hears what Rust sends to every other window too.
      getCurrentWebviewWindow().listen<AutomationNotice>(AUTOMATION_NOTIFY_EVENT, (e) => {
        const notice = e.payload;
        void invoke<boolean>('automations_notify_claim', { id: notice.id })
          .then((mine) => {
            if (mine) play(notice);
          })
          .catch(() => {});
      }),
    )
    .then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    stopped = true;
    unlisten?.();
  };
}

/** The same for a Speak action: the first window to claim the line speaks it.
 *  Linux builds list their voices late, so the probe starts with the window. */
export function startAutomationSpeak(): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  void speechAvailable();
  void Promise.resolve()
    .then(() =>
      // Window-scoped, like the notify card: a plain `listen` hears what Rust sends to every other window too.
      getCurrentWebviewWindow().listen<AutomationSpeak>(AUTOMATION_SPEAK_EVENT, (e) => {
        const line = e.payload;
        void invoke<boolean>('automations_notify_claim', { id: line.id })
          .then((mine) => {
            if (mine) void speak(line);
          })
          .catch(() => {});
      }),
    )
    .then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    stopped = true;
    unlisten?.();
  };
}

export function useAutomationNotify(): void {
  useEffect(() => {
    const stopNotify = startAutomationNotify();
    const stopSpeak = startAutomationSpeak();
    return () => {
      stopNotify();
      stopSpeak();
    };
  }, []);
}
