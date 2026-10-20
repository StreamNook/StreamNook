/**
 * Insert a StreamNook control into Plyr's control bar once it exists.
 *
 * Plyr's `controls` option only takes its built-ins, so app controls are
 * added to the DOM directly (Audio Boost was the first). The bar can appear
 * after the caller's effect runs, so this retries briefly, guards against
 * double insertion by `attr`, and returns a cancel function for cleanup. A
 * player rebuild replaces the whole bar, taking the control with it; the
 * caller re-runs on its "player ready" signal and this inserts a fresh one.
 */
export function injectPlyrControl(
  container: HTMLElement,
  opts: {
    /** Idempotency attribute, e.g. `data-streamnook-live`. */
    attr: string;
    className: string;
    /** Static markup only; anything dynamic is written afterwards. */
    html: string;
    onClick: () => void;
    /** Where to put it; return null to append. */
    place: (controls: Element) => { after: Element } | { before: Element } | null;
    onInserted?: (btn: HTMLButtonElement) => void;
  },
): () => void {
  let attempts = 0;
  let cancelled = false;
  const inject = () => {
    if (cancelled) return;
    const controls = container.querySelector('.plyr__controls');
    if (!controls) {
      if (attempts++ < 25) setTimeout(inject, 200);
      return;
    }
    if (controls.querySelector(`[${opts.attr}]`)) return;
    const btn = document.createElement('button');
    btn.className = `plyr__controls__item plyr__control ${opts.className}`;
    btn.type = 'button';
    btn.setAttribute(opts.attr, '');
    btn.innerHTML = opts.html;
    btn.addEventListener('click', opts.onClick);
    const where = opts.place(controls);
    if (where && 'after' in where && where.after.parentElement === controls) {
      where.after.insertAdjacentElement('afterend', btn);
    } else if (where && 'before' in where && where.before.parentElement === controls) {
      controls.insertBefore(btn, where.before);
    } else {
      controls.appendChild(btn);
    }
    opts.onInserted?.(btn);
  };
  inject();
  return () => {
    cancelled = true;
  };
}

/** The mouse events Plyr's seek bar reads a position from: where a press or
 *  drag lands (`seek-value`), its tooltip, and the storyboard preview. */
const SEEK_MOUSE_EVENTS = ['mousedown', 'mousemove', 'mouseenter', 'mouseleave'] as const;

/**
 * Hands Plyr's seek bar the pointer's exact position.
 *
 * Plyr works out where a press or drag lands, and the time its tooltip and
 * preview print, from mouse events, and Chromium rounds a mouse event's
 * coordinates to whole CSS pixels. The range under it follows the exact
 * pointer. At any display scale but 100% a screen pixel is a fraction of a
 * CSS pixel, so Plyr seeked up to a pixel short of the thumb, and each
 * finished seek pulled a held thumb back onto Plyr's number: a slow drag
 * flickered and landed a pixel off (tens of seconds on a long VOD).
 *
 * The pointer event dispatched just before each mouse event carries the
 * exact position; this copies it onto the mouse event, in the capture phase
 * on `root`, before Plyr's own listeners read it. Returns the cleanup.
 */
export function exactSeekPointer(root: HTMLElement): () => void {
  let last: { x: number; y: number } | null = null;
  const onPointer = (e: PointerEvent) => {
    if (e.isPrimary && e.isTrusted) last = { x: e.clientX, y: e.clientY };
  };
  const onMouse = (e: MouseEvent) => {
    const p = last;
    // A synthetic event (plyrFineScrub's) already carries its exact point.
    if (!p || !e.isTrusted || !(e.target instanceof Element) || !e.target.closest('.plyr__progress')) return;
    // Only the pointer event this mouse event is a rounded copy of.
    if (Math.abs(p.x - e.clientX) >= 1 || Math.abs(p.y - e.clientY) >= 1) return;
    const scrollX = e.pageX - e.clientX;
    const scrollY = e.pageY - e.clientY;
    Object.defineProperties(e, {
      clientX: { value: p.x },
      clientY: { value: p.y },
      pageX: { value: p.x + scrollX },
      pageY: { value: p.y + scrollY },
    });
  };
  root.addEventListener('pointerdown', onPointer, true);
  root.addEventListener('pointermove', onPointer, true);
  for (const type of SEEK_MOUSE_EVENTS) root.addEventListener(type, onMouse, true);
  return () => {
    root.removeEventListener('pointerdown', onPointer, true);
    root.removeEventListener('pointermove', onPointer, true);
    for (const type of SEEK_MOUSE_EVENTS) root.removeEventListener(type, onMouse, true);
  };
}
