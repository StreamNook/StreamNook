// The core's view of the app (services/app_presence.rs): read once at boot,
// then kept current by the `app-presence` event. The player asks
// isMainParked() synchronously on play: a parked relay means start over at
// the live edge, whatever the player's own pause clock says.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';

interface AppPresence {
  foreground: boolean;
  low_power: boolean;
  main_parked: boolean;
  parked_at: number | null;
}

let mainParked = false;

export function isMainParked(): boolean {
  return mainParked;
}

/** Test seam and the event handler's body. */
export function applyPresence(p: Pick<AppPresence, 'main_parked'>): void {
  mainParked = !!p.main_parked;
}

export function installAppPresence(): () => void {
  void invoke<AppPresence>('get_app_presence')
    .then(applyPresence)
    .catch((err) => Logger.debug('[AppPresence] get_app_presence failed:', err));
  const unlisten = listen<AppPresence>('app-presence', (e) => applyPresence(e.payload));
  return () => {
    void unlisten.then((f) => f());
  };
}
