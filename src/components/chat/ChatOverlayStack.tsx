import { type ReactNode } from 'react';

/**
 * Holds the cards at the top of chat (poll, prediction) in one column.
 *
 * Twitch lets a poll and a prediction run at the same time, so the cards stack
 * here rather than each pinning itself to the same box. Rust lays the column
 * out (chatTopStack.ts): the cards' order, the one that is open, and
 * `maxHeight`, past which the open card's body scrolls inside the column.
 *
 * Desktop floats it over chat at `top`; the phone leaves `top` out and the
 * column sits in the chat column's flow, so nothing can paint over a card.
 *
 * Empty renders to zero height, so it never sits over chat when nothing is live.
 */
interface ChatOverlayStackProps {
  /** Floating: where the column starts, just under the lowest piece of chat
   *  chrome (the header, a hype train inside it, the combined-chat bar),
   *  measured by the host so a taller header can never land a card on top of
   *  it. Left out, the column sits in the normal flow. */
  top?: number;
  /** The most height the cards may take, 0 for no cap yet. */
  maxHeight?: number;
  /** Receives the column, so the host can place what follows it. */
  stackRef?: (el: HTMLDivElement | null) => void;
  /** Hide the cards (they stay mounted) while another view replaces chat. */
  hidden?: boolean;
  className?: string;
  children: ReactNode;
}

export function ChatOverlayStack({
  top,
  maxHeight = 0,
  stackRef,
  hidden = false,
  className = '',
  children,
}: ChatOverlayStackProps) {
  const floating = top !== undefined;
  return (
    <div
      ref={stackRef}
      className={`${
        floating ? 'absolute left-2 right-2 z-40 transition-[top] duration-300 ease-in-out' : 'relative shrink-0'
      } flex-col gap-1 ${hidden ? 'hidden' : 'flex'} ${className}`}
      style={{ top, maxHeight: maxHeight > 0 ? maxHeight : undefined }}
    >
      {children}
    </div>
  );
}

export default ChatOverlayStack;
