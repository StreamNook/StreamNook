// Chats' close: the strip's own 40 px glass circle (as + beside it) with the
// X the offline room's corner uses, so every way out of a chat reads the same.
// Leaving Chats keeps its chats; the grabber and a slide down still work.
import React from 'react';
import { X } from 'phosphor-react';

export const ChatsCloseButton: React.FC<{ onClose: () => void }> = ({ onClose }) => (
  <button
    onClick={onClose}
    className="shrink-0 w-11 h-11 flex items-center justify-center active:opacity-70"
    aria-label="Close chats"
  >
    <span className="w-10 h-10 flex items-center justify-center rounded-full chrome-glaze chrome-glaze--flat text-textSecondary">
      <X size={14} weight="bold" />
    </span>
  </button>
);
