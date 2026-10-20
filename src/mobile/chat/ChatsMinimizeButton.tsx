// Chats' way out: a back arrow at the left edge of the row, where people look
// for "back" (Brandon's call). The room pills keep their own X for closing a
// room, so the row has one X. Leaving Chats keeps its chats; the grabber and a
// slide down still work.
import React from 'react';
import { CaretLeft } from 'phosphor-react';

export const ChatsMinimizeButton: React.FC<{ onMinimize: () => void; className?: string }> = ({
  onMinimize,
  className = '',
}) => (
  <button
    onClick={onMinimize}
    className={`shrink-0 w-11 h-11 flex items-center justify-center active:opacity-70 ${className}`}
    aria-label="Back"
    // Pressed while typing it still leaves Chats on that tap; the keyboard goes with it.
    data-taps-through-keyboard=""
  >
    {/* The strip's own 40 px glass circle, as + and play beside the rooms. A
        chevron's weight sits on its point side; nudged so it reads centered. */}
    <span className="w-10 h-10 flex items-center justify-center rounded-full chrome-glaze chrome-glaze--flat text-textSecondary">
      <CaretLeft size={15} weight="bold" className="-translate-x-[0.5px]" />
    </span>
  </button>
);
