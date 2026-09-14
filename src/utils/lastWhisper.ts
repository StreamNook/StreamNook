// Who whispered us last, for `/r <message>` (reply to the last whisper).
// One process-wide listener on the Rust `whisper-received` event; installed
// lazily by the command handler so windows that never run commands pay
// nothing.

import { listen } from '@tauri-apps/api/event';
import { Logger } from './logger';

let lastFrom: { login: string; name: string } | null = null;
let installed = false;

export function ensureLastWhisperListener(): void {
  if (installed) return;
  installed = true;
  void listen<{ from_user_login?: string; from_user_name?: string }>('whisper-received', (e) => {
    const login = e.payload?.from_user_login;
    if (login) lastFrom = { login, name: e.payload?.from_user_name || login };
  }).catch((err) => Logger.debug('[LastWhisper] listen failed:', err));
}

export function getLastWhisperFrom(): { login: string; name: string } | null {
  return lastFrom;
}
