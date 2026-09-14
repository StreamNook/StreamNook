// Bring the main app window back when a MultiChat popout needs it.
//
// Going live DESTROYS the main window to free its memory, so the popout-to-main
// handoffs (badge/paint overlay, public profile viewer, whisper, watch-in-main,
// clip/VOD playback, the "open main app" button) can no longer assume it exists.
// `ensureMainAlive` get-or-creates it (Rust `ensure_main_window`) and, when it had
// to be RECREATED, waits for the new window's listeners to register (`main-ready`)
// before the caller emits — otherwise the event would fire into a window that isn't
// listening yet and silently vanish. When main is merely hidden (or already open)
// this is a fast show+focus with no wait.

import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

// Generous cold-start ceiling: recreating main boots the whole app shell. The
// fallback only exists so a missed/late `main-ready` can't deadlock the action.
const READY_TIMEOUT_MS = 12_000;

async function mainWindowExists(): Promise<boolean> {
  try {
    const { getAllWindows } = await import('@tauri-apps/api/window');
    return (await getAllWindows()).some((w) => w.label === 'main');
  } catch {
    return false;
  }
}

/** Ensure the main window exists and is shown. Resolves once it's ready to receive
 *  events: immediately if it already existed (open or hidden), or after the
 *  recreated window emits `main-ready`. */
export async function ensureMainAlive(): Promise<void> {
  if (await mainWindowExists()) {
    // Already there — just bring it forward (Rust show); its listeners are live.
    await invoke('ensure_main_window').catch((e) => Logger.warn('[ensureMain] show failed:', e));
    return;
  }
  // Recreate from JS using THIS window's origin — the same way the popouts load
  // themselves, which Tauri recognizes as an app URL (so the Tauri API is injected)
  // in both dev (the Vite server) and prod (the asset protocol). Recreating Rust-side
  // pointed at the bundled dist renders blank in `tauri dev` (dist isn't built), and
  // an External URL gets no Tauri API — both showed a white window.
  const { listen } = await import('@tauri-apps/api/event');
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  await new Promise<void>((resolve) => {
    let settled = false;
    let unlisten: (() => void) | undefined;
    const finish = () => {
      if (settled) return;
      settled = true;
      try {
        unlisten?.();
      } catch {
        /* ignore */
      }
      resolve();
    };
    // Arm the readiness listener BEFORE creating the window, so a fast boot can't
    // emit `main-ready` before we're listening.
    void listen('main-ready', () => finish()).then((un) => {
      unlisten = un;
      try {
        const win = new WebviewWindow('main', {
          url: `${window.location.origin}/`,
          title: 'StreamNook',
          width: 1600,
          height: 1000,
          minWidth: 800,
          minHeight: 600,
          center: true,
          resizable: true,
          decorations: false,
          // Same hidden-until-first-paint gate as the config-defined window:
          // App's mount effect restores saved geometry and reveals, so the
          // recreated main appears painted instead of as a blank shell.
          visible: false,
          backgroundColor: '#0c0c0d',
        });
        win.once('tauri://error', (e) => {
          Logger.error('[ensureMain] create main failed:', e);
          finish();
        });
        // Popout-side net for the gate: if the new window's JS dies before its
        // own reveal, this shows it anyway (the command is idempotent and this
        // popout is guaranteed alive to send it).
        setTimeout(() => {
          void import('@tauri-apps/api/core')
            .then(({ invoke }) => invoke('reveal_main_window'))
            .catch(() => {});
        }, 3000);
      } catch (e) {
        Logger.error('[ensureMain] create main threw:', e);
        finish();
      }
      setTimeout(finish, READY_TIMEOUT_MS);
    });
  });
}

/** Ensure main is alive + ready, then emit a popout->main event to it. */
export async function ensureMainAndEmit(event: string, payload?: unknown): Promise<void> {
  await ensureMainAlive();
  const { emit } = await import('@tauri-apps/api/event');
  await emit(event, payload ?? {});
}
