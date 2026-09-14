// Scroll a chat list to a specific message and leave it centred.
//
// Used by the mobile shell so tapping a reply's "replying to" line jumps to the
// message being replied to.
//
// WHY THIS IS NOT SHARED WITH DESKTOP: ChatWidget has its own inline copy of
// this math (`scrollToMessage`). Extracting that and having both shells import
// this would be the tidier repo, but the desktop build is FROZEN and that rule
// explicitly outranks code reuse, so its copy is deliberately left alone. If
// desktop ever unfreezes, collapse the two - do not "fix" the duplication by
// editing ChatWidget in the meantime.
//
// Container-aware scrolling, NOT scrollIntoView. In a WebView scrollIntoView can
// scroll the whole document rather than just the list, which on desktop shifts
// the custom title bar and on a phone shifts the entire app frame under the
// status bar. Desktop's copy carries the same warning; it was learned once
// already.

export interface ScrollToMessageOptions {
  /** Where the message lands in the viewport. Default 'center'. */
  align?: 'start' | 'center' | 'end';
  /** Smooth glide vs instant jump. Default true. */
  smooth?: boolean;
}

function targetFor(
  container: HTMLElement,
  el: HTMLElement,
  align: 'start' | 'center' | 'end',
): number {
  // Rect-based rather than offsetTop: offsetTop is relative to the nearest
  // positioned ancestor, which is not guaranteed to be the scroller. Rects are
  // measured against the viewport, so the difference is always the true offset.
  const cRect = container.getBoundingClientRect();
  const eRect = el.getBoundingClientRect();
  const offsetWithin = eRect.top - cRect.top + container.scrollTop;

  let target: number;
  if (align === 'start') {
    target = offsetWithin;
  } else if (align === 'end') {
    target = offsetWithin - container.clientHeight + eRect.height;
  } else {
    target = offsetWithin - container.clientHeight / 2 + eRect.height / 2;
  }
  return Math.max(0, Math.min(target, container.scrollHeight - container.clientHeight));
}

/**
 * Scroll to the row carrying `data-message-id="<messageId>"`.
 *
 * Returns false when the message is no longer in the buffer or has not been
 * rendered, so the caller can tell the user rather than appearing to do nothing.
 *
 * The caller is expected to PAUSE the chat first. Without that, the next
 * incoming message re-pins the list to the bottom and undoes the jump.
 */
export function scrollChatToMessage(
  messageId: string,
  options?: ScrollToMessageOptions,
): boolean {
  const { align = 'center', smooth = true } = options ?? {};

  const el = document.querySelector<HTMLElement>(
    `[data-message-id="${CSS.escape(messageId)}"]`,
  );
  if (!el) return false;

  const container = el.closest<HTMLElement>('.overflow-y-auto');
  if (!container) return false;

  const behavior: ScrollBehavior = smooth ? 'smooth' : 'auto';
  container.scrollTo({ top: targetFor(container, el, align), behavior });

  // One correction pass, and it is not optional on mobile.
  //
  // Chat rows use `content-visibility: auto` with a `contain-intrinsic-block-size`
  // placeholder, so rows that were off screen when we measured were measured at
  // their GUESSED height. Once the scroll brings them into view their real
  // heights resolve, every row below shifts, and the target drifts off centre -
  // further the longer the jump. Re-measuring after layout settles fixes it.
  window.setTimeout(() => {
    const again = document.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(messageId)}"]`,
    );
    const c = again?.closest<HTMLElement>('.overflow-y-auto');
    if (!again || !c) return;
    const target = targetFor(c, again, align);
    // Only correct a drift worth correcting; a second glide over a few pixels
    // reads as a wobble.
    if (Math.abs(c.scrollTop - target) > 8) {
      c.scrollTo({ top: target, behavior });
    }
  }, 260);

  return true;
}
