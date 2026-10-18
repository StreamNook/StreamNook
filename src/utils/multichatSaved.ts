// MultiChat's saved tabs and layout, kept by Rust (commands/multichat_saved.rs)
// so they outlive the webview's storage. MultiChat is the desktop's chats with
// no stream playing, as the Chats tab is on the phones.
//
// The window keeps reading its fast copy from window storage before its first
// frame. Before the window's module loads, a missing copy is restored from
// Rust; every save goes to both, Rust's a moment later.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

const STORAGE_PREFIX = 'streamnook.multichat.';
const SAVE_DELAY_MS = 1000;

const pending = new Map<string, ReturnType<typeof setTimeout>>();

/** The window id in a MultiChat hash (`#/multichat?id=<id>&...`). */
export function multichatWindowId(hash: string): string | null {
  const query = hash.split('?')[1] ?? '';
  return new URLSearchParams(query).get('id');
}

/** Puts the saved state back into window storage when it is missing there. */
export async function restoreSavedMultichat(hash: string = window.location.hash): Promise<void> {
  const id = multichatWindowId(hash);
  if (!id) return;
  const key = `${STORAGE_PREFIX}${id}`;
  try {
    if (localStorage.getItem(key)) return;
    const saved = await invoke<unknown>('get_multichat_window_state', { windowId: id });
    if (saved != null) localStorage.setItem(key, JSON.stringify(saved));
  } catch (err) {
    Logger.debug('[MultiChat] no saved window state:', err);
  }
}

/** Saves the window's state to Rust, a moment after the last change. */
export function saveMultichatState(windowId: string, state: unknown): void {
  const prev = pending.get(windowId);
  if (prev) clearTimeout(prev);
  pending.set(
    windowId,
    setTimeout(() => {
      pending.delete(windowId);
      void invoke('save_multichat_window_state', { windowId, state }).catch((err) =>
        Logger.debug('[MultiChat] save failed:', err),
      );
    }, SAVE_DELAY_MS),
  );
}
