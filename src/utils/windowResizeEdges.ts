/**
 * How much of a borderless window's edge belongs to resizing rather than to
 * the page, per platform. Anything that reacts to the pointer at the window's
 * edge (the title bar's reveal at the top) has to reach further in than this,
 * or the pointer never gets to it.
 */

/** LinuxResizeEdges: the strip along each side that hands a drag to the
 *  window system's resize, in CSS pixels. It sits over everything. */
export const LINUX_RESIZE_EDGE_PX = 6;

/** LinuxResizeEdges: the square at each corner, in CSS pixels. */
export const LINUX_RESIZE_CORNER_PX = 12;

/**
 * Windows: Tauri gives an undecorated, resizable window a native child window
 * along its top edge (SM_CYFRAME tall, about 4 CSS pixels at any display
 * scale) that answers the resize hit-test. It sits above the webview in every
 * state except maximized, full screen included, so the page never sees the
 * pointer in those pixels.
 */
export const WINDOWS_TOP_RESIZE_BORDER_PX = 4;
