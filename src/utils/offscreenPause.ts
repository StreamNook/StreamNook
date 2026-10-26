// Marks an element scrolled out of view with `data-sn-off`, so the stylesheet
// can hold its infinite animation still (globals.css chat rows, mobile.css
// live-dot pings). One observer for every element; root = viewport, which
// IntersectionObserver clips by every scrolling ancestor.
let observer: IntersectionObserver | null = null;

function getObserver(): IntersectionObserver | null {
  if (observer) return observer;
  if (typeof IntersectionObserver === 'undefined') return null;
  observer = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        const el = e.target as HTMLElement;
        if (e.isIntersecting) delete el.dataset.snOff;
        else el.dataset.snOff = 'true';
      }
    },
    { rootMargin: '64px' },
  );
  return observer;
}

export function observeOffscreen(el: HTMLElement): () => void {
  const io = getObserver();
  if (!io) return () => {};
  io.observe(el);
  return () => {
    io.unobserve(el);
    delete el.dataset.snOff;
  };
}

/** Test seam. */
export function resetOffscreenObserver(): void {
  observer?.disconnect();
  observer = null;
}
