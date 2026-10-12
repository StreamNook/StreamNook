// Whether something on screen answers Esc itself, so Esc must not also leave
// the player's full screen.
//
// Two kinds of owner. A popup that is open and on top: a menu, dialog or list
// (role, or aria-modal) the viewer can actually see, not one laid out under the
// full-screen video or kept hidden in the page (Plyr keeps its settings menu
// mounted). And a trigger reporting its menu open (aria-expanded) on one of the
// surfaces that show over the full-screen player: the player itself (Plyr's
// settings button, the bar's menus' triggers), its chat column and the title
// bar, which holds the Island's door. Text fields never get here: the key
// dispatcher leaves bare keys to them.
//
// Pure over a small slice of the DOM so it can be tested without one.

const POPUPS = '[role="menu"], [role="dialog"], [role="alertdialog"], [role="listbox"], [aria-modal="true"]';
const OPEN_TRIGGERS = '[aria-expanded="true"]';
const SURFACES_OVER_PLAYER = '[data-titlebar], .plyr--fullscreen-fallback, .chat-fullscreen-overlay';

interface OwnerElement {
  closest(selectors: string): unknown;
  contains(other: unknown): boolean;
  getClientRects(): { length: number };
  getBoundingClientRect(): { left: number; top: number; width: number; height: number };
}

interface OwnerDocument {
  querySelectorAll(selectors: string): ArrayLike<OwnerElement>;
  elementFromPoint(x: number, y: number): unknown;
}

const shown = (el: OwnerElement): boolean => el.getClientRects().length > 0;

/** The popup is what the viewer sees at its own centre, not something above it. */
const onTop = (doc: OwnerDocument, el: OwnerElement): boolean => {
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) return false;
  const hit = doc.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return hit != null && (hit === el || el.contains(hit));
};

export function escapeHasOwner(doc: OwnerDocument | undefined = typeof document === 'undefined' ? undefined : document): boolean {
  if (!doc) return false;
  const triggers = doc.querySelectorAll(OPEN_TRIGGERS);
  for (let i = 0; i < triggers.length; i++) {
    const el = triggers[i];
    if (el.closest(SURFACES_OVER_PLAYER) && shown(el)) return true;
  }
  const popups = doc.querySelectorAll(POPUPS);
  for (let i = 0; i < popups.length; i++) {
    const el = popups[i];
    if (shown(el) && onTop(doc, el)) return true;
  }
  return false;
}
