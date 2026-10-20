// The phone's one rule for the keyboard (the iPhone's is SNKit OutsideTapRule):
// a tap outside the field being typed in puts the keyboard away, and that tap
// presses nothing else, so the chat row or the video under the finger is not
// pressed by the same touch. Where a tap is plainly meant for what it lands on
// (a search result, a sheet's buttons and switches) the keyboard still goes
// and the press lands too. A drag never closes it (people scroll chat while
// they type), except a drag on the chat list that reaches the keyboard's top
// edge: that pulls the keyboard down, as in Messages.
//
// Pages only mark themselves:
// - data-chat-composer (or data-keeps-keyboard): the field's own controls; a
//   press here acts and the keyboard stays.
// - data-taps-through-keyboard: a press here acts and the keyboard goes.
// - data-chat-list: the list a drag down into the keyboard pulls it from.
// Pop-outs close on a tap outside them through useOutsidePress, which on the
// phone follows the same tap rule.

/** How far a finger may move and still be a tap (CSS px). */
export const TAP_SLOP = 10;

const TEXT_TYPES = new Set(['text', 'search', 'email', 'url', 'tel', 'number', 'password']);

/** The field being typed in, when one is (the soft keyboard is up for it).
 *  Read by tag, so the rule runs in tests without a DOM. */
export function editingField(active: Element | null): HTMLElement | null {
  const el = active as HTMLElement | null;
  if (!el) return null;
  if (el.tagName === 'TEXTAREA' || el.isContentEditable) return el;
  if (el.tagName === 'INPUT' && TEXT_TYPES.has((el as HTMLInputElement).type)) return el;
  return null;
}

/** The pressed element, when the event's target is one. */
function elementOf(target: EventTarget | null): Element | null {
  return target && typeof (target as Element).closest === 'function' ? (target as Element) : null;
}

export type KeyboardTap =
  /** Nothing to put away, or the press is the field's own. */
  | 'none'
  /** The keyboard goes and the press stops there. */
  | 'use'
  /** The keyboard goes and the press lands too. */
  | 'pass';

/** What a press on `target` does to the keyboard while `field` has it. */
export function keyboardTap(target: Element, field: Element | null): KeyboardTap {
  if (!field || field.contains(target)) return 'none';
  if (target.closest('[data-chat-composer], [data-keeps-keyboard]')) return 'none';
  return target.closest('[data-taps-through-keyboard]') ? 'pass' : 'use';
}

/** Whether a press that moved `dx`, `dy` is still a tap. */
export function isTap(dx: number, dy: number): boolean {
  return Math.hypot(dx, dy) <= TAP_SLOP;
}

/** The soft keyboard's top edge in the viewport, null while it is down. The
 *  app draws edge to edge, so the viewport never shrinks for it; the native
 *  bridge reports its height as --sn-kb (less the gesture bar, --sn-safe-b). */
function cssKeyboardTop(doc: Document): number | null {
  const win = doc.defaultView;
  if (!win) return null;
  const css = win.getComputedStyle(doc.documentElement);
  const kb = parseFloat(css.getPropertyValue('--sn-kb')) || 0;
  if (kb <= 0) return null;
  const safe = parseFloat(css.getPropertyValue('--sn-safe-b')) || 0;
  return win.innerHeight - kb - safe;
}

/**
 * Listens on the document for the keyboard rule. Returns the cleanup. Mounted
 * once (KeyboardDismiss, beside the tab bar every screen keeps).
 */
export function installKeyboardTaps(
  doc: Document = document,
  keyboardTop: () => number | null = () => cssKeyboardTop(doc),
): () => void {
  let press: { id: number; x: number; y: number; verdict: KeyboardTap; field: HTMLElement } | null = null;
  // A drag on the chat list while typing: where it started, to see it head
  // down. Touch events, since a scroll cancels the pointer stream.
  let drag: { y: number; field: HTMLElement } | null = null;

  const onDown = (e: PointerEvent) => {
    press = null;
    const field = editingField(doc.activeElement);
    const target = elementOf(e.target);
    if (!field || !target) return;
    const verdict = keyboardTap(target, field);
    if (verdict !== 'none') press = { id: e.pointerId, x: e.clientX, y: e.clientY, verdict, field };
  };
  const onMove = (e: PointerEvent) => {
    if (press && press.id === e.pointerId && !isTap(e.clientX - press.x, e.clientY - press.y)) press = null;
  };
  const onTouchStart = (e: TouchEvent) => {
    drag = null;
    const field = editingField(doc.activeElement);
    const t = e.touches[0];
    if (field && t && e.touches.length === 1 && elementOf(e.target)?.closest('[data-chat-list]')) {
      drag = { y: t.clientY, field };
    }
  };
  const onTouchMove = (e: TouchEvent) => {
    const t = e.touches[0];
    if (!drag || !t || t.clientY <= drag.y) return;
    const top = keyboardTop();
    if (top != null && t.clientY >= top) {
      drag.field.blur();
      drag = null;
    }
  };
  const onTouchEnd = () => {
    drag = null;
  };
  const onClick = (e: MouseEvent) => {
    const p = press;
    press = null;
    if (!p) return;
    p.field.blur();
    if (p.verdict === 'use') {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  const onCancel = (e: PointerEvent) => {
    // The page took the touch (a scroll): a drag, not a tap.
    if (press?.id === e.pointerId) press = null;
  };
  const passive = { capture: true, passive: true } as const;
  doc.addEventListener('pointerdown', onDown, true);
  doc.addEventListener('pointermove', onMove, true);
  doc.addEventListener('pointercancel', onCancel, true);
  doc.addEventListener('click', onClick, true);
  doc.addEventListener('touchstart', onTouchStart, passive);
  doc.addEventListener('touchmove', onTouchMove, passive);
  doc.addEventListener('touchend', onTouchEnd, passive);
  doc.addEventListener('touchcancel', onTouchEnd, passive);
  return () => {
    doc.removeEventListener('pointerdown', onDown, true);
    doc.removeEventListener('pointermove', onMove, true);
    doc.removeEventListener('pointercancel', onCancel, true);
    doc.removeEventListener('click', onClick, true);
    doc.removeEventListener('touchstart', onTouchStart, passive);
    doc.removeEventListener('touchmove', onTouchMove, passive);
    doc.removeEventListener('touchend', onTouchEnd, passive);
    doc.removeEventListener('touchcancel', onTouchEnd, passive);
  };
}
