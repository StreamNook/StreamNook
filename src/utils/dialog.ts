// Every dialog the page opens goes through here (dialog.test.ts pins that this
// is the only importer of the dialog plugin). While one is open, Rust counts a
// dialog up for this window (`set_dialog_open`), so Performance mode's "In the
// background" never mistakes a StreamNook file picker or question, which on
// Linux belongs to another process, for another app taking the front. The
// phones have no such mode and skip the call.
import { invoke } from '@tauri-apps/api/core';
import type { ConfirmDialogOptions, OpenDialogOptions, OpenDialogReturn, SaveDialogOptions } from '@tauri-apps/plugin-dialog';
import { IS_MOBILE } from './platform';

function mark(open: boolean): Promise<void> {
  if (IS_MOBILE) return Promise.resolve();
  return invoke('set_dialog_open', { open }).then(
    () => {},
    () => {},
  );
}

async function held<T>(run: () => Promise<T>): Promise<T> {
  await mark(true);
  try {
    return await run();
  } finally {
    void mark(false);
  }
}

export function open<T extends OpenDialogOptions>(options?: T): Promise<OpenDialogReturn<T>> {
  return held(async () => (await import('@tauri-apps/plugin-dialog')).open(options));
}

export function save(options?: SaveDialogOptions): Promise<string | null> {
  return held(async () => (await import('@tauri-apps/plugin-dialog')).save(options));
}

export function ask(message: string, options?: string | ConfirmDialogOptions): Promise<boolean> {
  return held(async () => (await import('@tauri-apps/plugin-dialog')).ask(message, options));
}
