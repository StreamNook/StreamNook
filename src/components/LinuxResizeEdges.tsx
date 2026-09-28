// Resize borders for the undecorated windows on Linux.
//
// StreamNook's desktop windows are borderless and draw their own title bar.
// Windows gives a borderless window resize borders anyway (tao answers
// WM_NCHITTEST for them), and the WebKitGTK build had GTK's. The Linux build
// embeds Chromium in a plain X11 window, which has neither: nothing at the
// window's edge belongs to anyone, so the pointer never gets a resize cursor
// and a drag there does nothing. This is the border: eight invisible strips
// along the edges and corners that hand the drag to the runtime
// (`startResizeDragging`, which is the window system's own interactive
// resize, so it tracks the pointer exactly as a native border would).
//
// Page-side because it is a hit-test on the page's own pixels, which nothing
// under the page can see: the runtime has no edge hit-testing of its own.
// Mounted once per window by main.tsx, on Linux only. A maximized or
// full-screen window has no edges to drag, and a window that is not
// resizable gets none either.

import { useEffect, useState, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Logger } from '../utils/logger';
import type { WindowFlags } from '../utils/windowFrameState';

/** How many CSS pixels of the window's edge answer as the border. */
const EDGE = 6;
const CORNER = 12;

/** `startResizeDragging`'s argument; the API declares the type but does not export it. */
type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>['startResizeDragging']>[0];

type Edge = {
  direction: ResizeDirection;
  cursor: string;
  style: CSSProperties;
};

const EDGES: Edge[] = [
  { direction: 'North', cursor: 'n-resize', style: { top: 0, left: CORNER, right: CORNER, height: EDGE } },
  { direction: 'South', cursor: 's-resize', style: { bottom: 0, left: CORNER, right: CORNER, height: EDGE } },
  { direction: 'West', cursor: 'w-resize', style: { left: 0, top: CORNER, bottom: CORNER, width: EDGE } },
  { direction: 'East', cursor: 'e-resize', style: { right: 0, top: CORNER, bottom: CORNER, width: EDGE } },
  { direction: 'NorthWest', cursor: 'nw-resize', style: { top: 0, left: 0, width: CORNER, height: CORNER } },
  { direction: 'NorthEast', cursor: 'ne-resize', style: { top: 0, right: 0, width: CORNER, height: CORNER } },
  { direction: 'SouthWest', cursor: 'sw-resize', style: { bottom: 0, left: 0, width: CORNER, height: CORNER } },
  { direction: 'SouthEast', cursor: 'se-resize', style: { bottom: 0, right: 0, width: CORNER, height: CORNER } },
];

export default function LinuxResizeEdges() {
  const [active, setActive] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    // One round trip for the three flags (commands/window_state.rs): at boot
    // each plugin call waits behind whatever the page is doing, so three in a
    // row cost three waits.
    const refresh = () => {
      invoke<WindowFlags>('get_window_flags')
        .then(({ resizable, maximized, fullscreen }) => {
          if (!disposed) setActive(resizable && !maximized && !fullscreen);
        })
        .catch(() => {
          if (!disposed) setActive(false);
        });
    };
    refresh();
    const unlisten = win.onResized(refresh);
    return () => {
      disposed = true;
      void unlisten.then((off) => off()).catch(() => {});
    };
  }, []);

  if (!active) return null;

  return (
    <>
      {EDGES.map((edge) => (
        <div
          key={edge.direction}
          aria-hidden
          style={{ position: 'fixed', zIndex: 2147483647, cursor: edge.cursor, ...edge.style }}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            e.stopPropagation();
            getCurrentWindow()
              .startResizeDragging(edge.direction)
              .catch((err) => Logger.warn('[LinuxResizeEdges] resize drag refused:', err));
          }}
        />
      ))}
    </>
  );
}
