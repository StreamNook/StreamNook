// The phone's way down for the keyboard, everywhere at once. Scrolling never
// closes it (people scroll chat while they type), so:
// - A tap outside the field puts it away (outsideTap.ts), the tap pressing
//   nothing else except where it is plainly meant for what it lands on; a
//   drag on the chat list that reaches the composer pulls it down.
// - Enter in a single-line field puts the keyboard away once the field has
//   done its own work with it (search, add, save); Android's keys otherwise
//   leave it up.
// - A multi-line field, where Enter adds a line, gets a hide-keyboard pill
//   riding the top of the keyboard while it has focus. A field with its own
//   button (the chat composer) opts out with data-keyboard-hide="own".
// Mounted once, beside the tab bar, which every screen keeps mounted.
import React, { useEffect, useState } from 'react';
import { KeyboardHideIcon } from './KeyboardHideIcon';
import { installKeyboardTaps } from './outsideTap';

const SINGLE_LINE = new Set(['text', 'search', 'email', 'url', 'tel', 'number', 'password']);

export const KeyboardDismiss: React.FC = () => {
  const [editor, setEditor] = useState<HTMLTextAreaElement | null>(null);

  useEffect(() => installKeyboardTaps(), []);

  useEffect(() => {
    // Bubble phase, after the field's own Enter handling: a handler that
    // prevented the default (the composer sending) keeps the keyboard.
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target;
      if (e.key !== 'Enter' || e.defaultPrevented || e.isComposing) return;
      if (target instanceof HTMLInputElement && SINGLE_LINE.has(target.type)) target.blur();
    };
    const onFocusIn = (e: FocusEvent) => {
      const target = e.target;
      setEditor(target instanceof HTMLTextAreaElement && target.dataset.keyboardHide !== 'own' ? target : null);
    };
    const onFocusOut = () => setEditor(null);
    document.addEventListener('keydown', onKeyDown);
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('focusout', onFocusOut);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('focusin', onFocusIn);
      document.removeEventListener('focusout', onFocusOut);
    };
  }, []);

  if (!editor) return null;
  return (
    <button
      type="button"
      // Held on the press, so the field keeps the keyboard until the tap lands.
      onPointerDown={(e) => e.preventDefault()}
      onClick={() => editor.blur()}
      aria-label="Hide keyboard"
      className="sn-touch fixed right-3 z-[9100] flex h-9 w-11 items-center justify-center rounded-full chrome-glaze text-textSecondary active:opacity-70"
      style={{ bottom: 'calc(var(--sn-kb, 0px) + var(--sn-safe-b, 0px) + 8px)' }}
    >
      <KeyboardHideIcon size={19} />
    </button>
  );
};
