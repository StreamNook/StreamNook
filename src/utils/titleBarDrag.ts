import type React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from './logger';

/**
 * Pixels the pointer must travel before a MAXIMIZED window is handed to the OS drag
 * loop. Windows defers the restore the same way on a native caption, and it is what
 * keeps double-click-to-restore working: the second click of a double-click never
 * travels far enough to trigger a drag, so it reaches the `detail === 2` branch below.
 *
 * A window that is already restored skips the threshold entirely and hands off on
 * mousedown, matching what `data-tauri-drag-region` did. Waiting for movement there
 * would start the OS loop from wherever the pointer had already reached, so a fast
 * flick would leave the window held some distance from where it was grabbed.
 */
const DRAG_THRESHOLD_PX = 5;

/**
 * True from the drag hand-off until shortly after the OS modal move loop ends.
 * A setSize issued inside that loop corrupts its cached rect and commits a bogus
 * size on mouse-up, so the aspect-ratio lock checks this before resizing. The
 * Rust side polls the mouse button and emits `titlebar-drag-ended` on release
 * (with a 120s deadline, mirrored by the safety cap below); the 300ms grace on
 * the event covers the resize listener's trailing debounce.
 */
let dragActiveUntil = 0;
export const isTitlebarDragActive = () => Date.now() < dragActiveUntil;

void listen('titlebar-drag-ended', () => {
  dragActiveUntil = Date.now() + 300;
});

const beginDrag = () => {
  dragActiveUntil = Date.now() + 120_000;
  invoke('start_titlebar_drag').catch((error) => {
    dragActiveUntil = 0;
    Logger.error('[TitleBar] Failed to start window drag:', error);
  });
};

/**
 * Title bar drag handler, replacing `data-tauri-drag-region` on the main window.
 *
 * Mirrors Tauri's own drag-region script (exact element match so child controls stay
 * clickable, left button only, maximize on the second mousedown) and adds the
 * restore-from-maximized step Windows does not perform for a borderless window.
 * The restore itself happens Rust-side in one IPC hop: every extra hop between the
 * mousedown and `start_dragging` is a chance for the mouse button to come up first,
 * which leaves the window stuck to the cursor.
 *
 * `isMaximized` is the caller's tracked state rather than a fresh `isMaximized()`
 * call so the common path costs no IPC round trip. A stale value only picks the
 * wrong hand-off timing; the Rust side re-checks before restoring either way.
 */
export const handleTitleBarMouseDown = (
  event: React.MouseEvent<HTMLElement>,
  isMaximized: boolean,
) => {
  if (event.button !== 0) return;
  if (event.target !== event.currentTarget) return;
  if (event.detail > 2) return;
  event.preventDefault();

  if (event.detail === 2) {
    import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => getCurrentWindow().toggleMaximize())
      .catch((error) => Logger.error('[TitleBar] Failed to toggle maximize:', error));
    return;
  }

  if (!isMaximized) {
    beginDrag();
    return;
  }

  const startX = event.screenX;
  const startY = event.screenY;

  const onMove = (move: MouseEvent) => {
    if (
      Math.abs(move.screenX - startX) < DRAG_THRESHOLD_PX &&
      Math.abs(move.screenY - startY) < DRAG_THRESHOLD_PX
    ) {
      return;
    }
    cleanup();
    beginDrag();
  };

  // `blur` is in the cleanup set because the OS drag loop steals focus, and a mouseup
  // consumed by that loop would otherwise leave these listeners attached.
  const cleanup = () => {
    window.removeEventListener('mousemove', onMove);
    window.removeEventListener('mouseup', cleanup);
    window.removeEventListener('blur', cleanup);
  };

  window.addEventListener('mousemove', onMove);
  window.addEventListener('mouseup', cleanup);
  window.addEventListener('blur', cleanup);
};
