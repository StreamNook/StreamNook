// Gives back the image resource an <img> element holds. A chat row that was
// trimmed away leaves its images referenced until a major garbage collection,
// which a steady JS heap triggers rarely, so the renderer's memory climbs on a
// busy chat. Removing the sources lets the decoded images go at once.

/** Drops an image's sources. With both gone no error event fires. */
export function releaseImage(img: HTMLImageElement): void {
  img.removeAttribute('srcset');
  img.removeAttribute('src');
}

/** Releases every image under `root`. */
export function releaseImagesIn(root: ParentNode): void {
  for (const img of root.querySelectorAll('img')) releaseImage(img);
}

/**
 * A ref callback for a chat row's wrapper. Its cleanup runs when React unmounts
 * the row, and releases the row's images once a microtask has confirmed the
 * element left the document (React's StrictMode runs the cleanup on mount in
 * development with the element still connected, and that row must keep its
 * images). Module-level, so its identity is stable and React runs the cleanup
 * only on unmount.
 */
export function releaseRowImagesOnUnmount(el: HTMLElement | null): (() => void) | undefined {
  if (!el) return undefined;
  return () => {
    queueMicrotask(() => {
      if (!el.isConnected) releaseImagesIn(el);
    });
  };
}
