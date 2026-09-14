import { isWindowHidden, onWindowVisibility } from './windowVisibility';
// Window-title flasher for highlight matches that arrive while the window is
// blurred. Replaces document.title with an attention-grabbing variant until the
// window regains focus, then restores the original. Idempotent on repeat
// triggers (only one flash interval runs at a time).

let originalTitle: string | null = null;
let flashTimer: number | null = null;
let isFlashing = false;
let focusListenerAttached = false;

const FLASH_PREFIX = '🔔 ';
const FLASH_INTERVAL_MS = 900;

function restoreTitle() {
  if (originalTitle !== null) {
    document.title = originalTitle;
    originalTitle = null;
  }
  if (flashTimer !== null) {
    window.clearInterval(flashTimer);
    flashTimer = null;
  }
  isFlashing = false;
}

function ensureFocusListener() {
  if (focusListenerAttached) return;
  focusListenerAttached = true;
  window.addEventListener('focus', restoreTitle);
  onWindowVisibility(() => {
    if (!isWindowHidden()) restoreTitle();
  });
}

/**
 * Trigger a title flash. No-op if:
 *  - Window already has focus (no point flashing a visible window)
 *  - Globally disabled via settings (caller checks this)
 *  - Already flashing (just lets the existing flash continue)
 */
export function flashTitle(label: string): void {
  if (typeof document === 'undefined' || typeof window === 'undefined') return;
  if (document.hasFocus() && !isWindowHidden()) return;
  ensureFocusListener();
  if (isFlashing) return;

  originalTitle = document.title;
  isFlashing = true;
  let toggled = false;
  // Set first toggle immediately so the user sees feedback instantly.
  document.title = `${FLASH_PREFIX}${label}`;
  flashTimer = window.setInterval(() => {
    toggled = !toggled;
    if (originalTitle === null) {
      restoreTitle();
      return;
    }
    document.title = toggled ? originalTitle : `${FLASH_PREFIX}${label}`;
  }, FLASH_INTERVAL_MS);
}

// Test-only export so a unit test can force-restore without needing to fake
// the focus event.
export const __testing = { restoreTitle };
