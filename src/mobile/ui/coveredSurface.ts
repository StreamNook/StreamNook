// The phone shell stays mounted under the full player (Chats keeps its composer, its menus and its
// scroll), and the player's own chat stays mounted inside the mini box. Out of sight, a surface
// must neither keep the keyboard nor take the visible layer's taps.

/** A container out of sight for input: the shell under the full watch layer (from the layer's
 *  first frame), the player's chat while hidden. Separate from `data-sn-covered`, which rests the
 *  shell's glass and animations and so waits until the layer is actually opaque. */
export const OUT_OF_SIGHT_SELECTOR = '[data-sn-out-of-sight="true"]';

/** The focused element to blur when `surface` goes out of sight: `active` when it sits inside
 *  it, otherwise null. */
export function focusUnder<T>(surface: { contains(node: T): boolean } | null, active: T | null): T | null {
  if (!surface || !active) return null;
  return surface.contains(active) ? active : null;
}

/** While `surface` is out of sight, refuses any focus that arrives inside it (a sheet above it
 *  picking a command for the composer under the player). Returns the undo. A field's own focus
 *  handler still runs after the refusal, so it must check `document.activeElement` itself. */
export function refuseFocusIn(surface: EventTarget): () => void {
  const refuse = (e: Event) => {
    const target = e.target as { blur?: () => void } | null;
    target?.blur?.();
  };
  surface.addEventListener('focusin', refuse);
  return () => surface.removeEventListener('focusin', refuse);
}

/** Whether a menu is out of sight: every part of it that has an element sits in an out-of-sight
 *  container. A part portaled out (a sheet) keeps it in sight; no element at all is in sight. */
export function menuOutOfSight(parts: ReadonlyArray<{ closest(selector: string): unknown } | null | undefined>): boolean {
  const present = parts.filter((part): part is { closest(selector: string): unknown } => !!part);
  return present.length > 0 && present.every((part) => part.closest(OUT_OF_SIGHT_SELECTOR) !== null);
}

/** Whether a press outside a menu closes it. A menu out of sight (the Chats composer's emote
 *  panel under the full player) lets a press on the visible layer land where it was aimed; a press
 *  on the hidden surface itself still counts. */
export function closesCoveredMenu(menuCovered: boolean, pressCovered: boolean): boolean {
  return !menuCovered || pressCovered;
}

/** The full watch layer is up over the shell: full mode with a stream, one still loading, or an
 *  offline room (WatchScreen shows on `streamUrl || isLoading`). One rule for every reader. */
export function watchLayerUp(playerMode: 'full' | 'mini', streamUrl: string | null, isLoading: boolean): boolean {
  return playerMode === 'full' && (!!streamUrl || isLoading);
}

/** Which chat pane is on screen for the inbox: the player's while the layer is up (it sits over
 *  Chats), Chats' while it is not. A pane's own `idle` and `paused` rule it out when hidden anyway
 *  (PiP, landscape with chat away, Chats not in front); the mini box is ruled out by the layer. */
export function inboxPaneOnScreen(surface: 'watch' | 'chats', layerUp: boolean, idle: boolean, paused: boolean): boolean {
  return !paused && !idle && (surface === 'chats' ? !layerUp : layerUp);
}
