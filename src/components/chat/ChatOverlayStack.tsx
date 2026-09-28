import { type ReactNode } from 'react';

/**
 * Holds the floating chat overlays (poll, prediction) in one positioned column.
 *
 * Twitch lets a poll and a prediction run at the same time, and both overlays
 * used to pin themselves to the identical absolute box, so one silently painted
 * over the other. Stacking them in a flex column fixes that without measuring
 * anything: each card keeps its own collapsed/expanded height and the next one
 * follows underneath.
 *
 * Empty renders to zero height, so it never sits over chat when nothing is live.
 */
interface ChatOverlayStackProps {
  /** Where the column starts: just under the lowest piece of chat chrome (the
   *  header, a hype train inside it, the combined-chat bar), measured by the
   *  host so a taller header can never land a card on top of it. */
  top: number;
  /** Receives the column, so the host can place the pinned message under it. */
  stackRef?: (el: HTMLDivElement | null) => void;
  children: ReactNode;
}

export function ChatOverlayStack({ top, stackRef, children }: ChatOverlayStackProps) {
  return (
    <div
      ref={stackRef}
      className="absolute left-2 right-2 z-40 flex flex-col gap-2 transition-[top] duration-300 ease-in-out"
      style={{ top }}
    >
      {children}
    </div>
  );
}

export default ChatOverlayStack;
