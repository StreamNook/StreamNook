// A tap that changes what is under the finger (the mini box growing back into
// the full player) is followed by the click the WebView makes from it, and that
// click is aimed at whatever is there by then: the player's own buttons, still
// laid out in the small box. The WebView also nudges a click onto the nearest
// pressable element, so it can just as well land on a card beside the box.
// This eats that one click at the document, the way the keyboard rule in
// outsideTap.ts eats the tap that puts the keyboard away.

/** Longest the trailing click is waited for. It comes within a frame or two. */
export const TRAILING_CLICK_MS = 600;

/**
 * Swallows the next click on `doc`, unless a new press starts first (its own
 * click is a real one) or `ms` passes. Returns the disarm.
 */
export function swallowTrailingClick(doc: EventTarget = document, ms = TRAILING_CLICK_MS): () => void {
  // The options object, not a bare `true`: Node's EventTarget (the tests) does
  // not match a boolean capture flag on removal, and browsers take either.
  const capture = { capture: true } as const;
  let timer: ReturnType<typeof setTimeout> | null = setTimeout(disarm, ms);
  function onClick(e: Event) {
    e.preventDefault();
    e.stopPropagation();
    disarm();
  }
  function disarm() {
    doc.removeEventListener('click', onClick, capture);
    doc.removeEventListener('pointerdown', disarm, capture);
    if (timer) clearTimeout(timer);
    timer = null;
  }
  doc.addEventListener('click', onClick, capture);
  doc.addEventListener('pointerdown', disarm, capture);
  return disarm;
}
