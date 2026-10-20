// Chats' way out: a back arrow at the left edge of the row, where people look
// for "back" (Brandon's call). The room pills keep their own X for closing a
// room, so the row has one X. Leaving Chats keeps its chats; the grabber and a
// slide down still work.
import React from 'react';
import { CaretLeft } from 'phosphor-react';

// The player bar's icon size.
const ICON = 20;

export const ChatsMinimizeButton: React.FC<{ onMinimize: () => void; className?: string }> = ({
  onMinimize,
  className = '',
}) => (
  <button
    onClick={onMinimize}
    className={`shrink-0 w-11 h-11 flex items-center justify-center text-textSecondary active:opacity-70 ${className}`}
    aria-label="Back"
    // Pressed while typing it still leaves Chats on that tap; the keyboard goes with it.
    data-taps-through-keyboard=""
  >
    <CaretLeft size={ICON} />
  </button>
);
