// What covers the window's content, for anything that should stop working
// while nobody can see it: the immersive light (mediaGlow) and moving profile
// pictures (pictureVisibility). One definition, so the two can never disagree
// about what counts as covered.
//
// Kept structural rather than a list of store flags, because the overlays live
// in a dozen components and several keep their open state locally.

/** A full-window overlay: a `fixed inset-0` layer carrying a backdrop blur
 *  (Settings, the Badges, Drops, 7TV and Marketplace overlays, the command
 *  palette, the profile and clip modals). Same criterion as the
 *  `data-player-covered` rules in globals.css: a backdrop filter re-samples
 *  everything behind it whenever that content changes, so anything that moves
 *  underneath one is worse than wasted. */
export const OVERLAY_SELECTOR = '.fixed.inset-0[class*="backdrop-blur"]';

/** The player in full screen. Plyr is forced into its CSS full screen (the
 *  window is borderless), which pins `.plyr` over the whole window, so the
 *  sidebar, Home and the dock are all underneath it. */
export const FULLSCREEN_PLAYER_SELECTOR = '.plyr--fullscreen-fallback';

/** A container the app itself marks as covered by a full-screen layer (the
 *  phone shell under the watch screen, see MobileApp). Everything inside it is
 *  out of sight. */
export const COVERED_CONTAINER_SELECTOR = '[data-sn-covered="true"]';

/** Is a full-window overlay open? The immersive light's question: the player
 *  is never inside one of these. */
export function coveredByOverlay(root: ParentNode = document): boolean {
  return !!root.querySelector(OVERLAY_SELECTOR);
}

/** Every layer currently covering the window: the open overlays, the player in
 *  full screen and the element-level full-screen element, if any. Empty almost
 *  all the time, and one selector match when it is not. */
export function coveringLayers(doc: Document = document): Element[] {
  const layers = Array.from(doc.querySelectorAll(`${OVERLAY_SELECTOR}, ${FULLSCREEN_PLAYER_SELECTOR}`));
  const fullscreen = doc.fullscreenElement;
  if (fullscreen && !layers.includes(fullscreen)) layers.push(fullscreen);
  return layers;
}

/** Whether a layer is still covering: still in the document and still
 *  matching (Plyr drops its full-screen class on the same node it keeps). */
export function layerStillCovers(layer: Element, doc: Document = document): boolean {
  if (!layer.isConnected) return false;
  return layer === doc.fullscreenElement || layer.matches(`${OVERLAY_SELECTOR}, ${FULLSCREEN_PLAYER_SELECTOR}`);
}

/** Minimal shape the decision needs, so it is testable without a DOM. */
export interface StackNode {
  contains(other: StackNode | null): boolean;
}

/**
 * Is `target` hidden under one of `layers`, given `stack`: the elements at the
 * target's centre, topmost first (`document.elementsFromPoint`)?
 *
 * Walks down from the top. Reaching the target, or anything that holds it,
 * first means nothing covers it there (it lives inside the top overlay, or in
 * a layer drawn above it, like the full-screen chat column over the player).
 * Reaching part of a covering layer first means that layer is drawn over it.
 * With no layer anywhere in the stack, nothing covers it.
 */
export function hiddenUnderLayer<T extends StackNode>(
  stack: readonly T[],
  target: T,
  layers: readonly T[],
): boolean {
  if (layers.length === 0) return false;
  for (const hit of stack) {
    if (hit === target || hit.contains(target)) return false;
    if (layers.some((layer) => layer === hit || layer.contains(hit))) return true;
  }
  return false;
}
