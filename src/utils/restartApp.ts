import { invoke } from '@tauri-apps/api/core';
import { captureResumeSnapshot } from '../services/sessionResume';

/** Restart StreamNook. The stream on screen is saved first, so the new run
 *  opens it again. `reason` lands in the exit log. A dev build reloads the page
 *  instead, which keeps the `tauri dev` server (and does not apply browser
 *  switches, which only a real restart does). */
export async function restartApp(reason: string): Promise<void> {
  await captureResumeSnapshot();
  if (import.meta.env.DEV) {
    window.location.reload();
    return;
  }
  await invoke('restart_app', { reason });
}
