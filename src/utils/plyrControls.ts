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
