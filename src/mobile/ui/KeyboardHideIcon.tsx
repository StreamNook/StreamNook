// The hide-keyboard glyph: a keyboard over a down chevron, the mark Android's
// own keyboards use for "put the keyboard away" (and the iPhone draws as
// keyboard.chevron.compact.down). Scrolling never closes the keyboard, so a
// field that needs a way down shows this. Drawn in the current text color.
import React from 'react';

export const KeyboardHideIcon: React.FC<{ size?: number; className?: string }> = ({ size = 19, className }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden className={className}>
    <path d="M20 3H4c-1.1 0-1.99.9-1.99 2L2 15c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-9 3h2v2h-2V6zm0 3h2v2h-2V9zM8 6h2v2H8V6zm0 3h2v2H8V9zm-1 2H5V9h2v2zm0-3H5V6h2v2zm9 7H8v-2h8v2zm0-4h-2V9h2v2zm0-3h-2V6h2v2zm3 3h-2V9h2v2zm0-3h-2V6h2v2zm-7 15l4-4H8l4 4z" />
  </svg>
);
