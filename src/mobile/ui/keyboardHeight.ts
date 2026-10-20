// The soft keyboard's last height, so the emote panel can take the keyboard's
// place at the keyboard's size (the platform's own emoji-keyboard pattern):
// swapping one for the other leaves the composer where it was. The native
// inset bridge reports the live height as --sn-kb (less the gesture bar); the
// last open height is kept here, and across launches.
import { useEffect, useState } from 'react';

const KEY = 'sn-last-kb';
/** Before any keyboard has opened: a common phone keyboard's height. */
export const DEFAULT_KEYBOARD_PX = 300;
/** Below this a reading is a toolbar or a closing frame, not a keyboard. */
const MIN_KEYBOARD_PX = 120;

/** The height to remember after reading `kb`: a real keyboard's, else the last one. */
export function rememberedHeight(previous: number, kb: number): number {
  return kb >= MIN_KEYBOARD_PX ? Math.round(kb) : previous;
}

function stored(): number {
  try {
    const n = Number(localStorage.getItem(KEY));
    return Number.isFinite(n) && n >= MIN_KEYBOARD_PX ? n : DEFAULT_KEYBOARD_PX;
  } catch {
    return DEFAULT_KEYBOARD_PX;
  }
}

/** The last keyboard height in px, following --sn-kb as the keyboard opens. */
export function useLastKeyboardHeight(): number {
  const [height, setHeight] = useState(stored);
  useEffect(() => {
    const root = document.documentElement;
    const read = () => {
      const kb = parseFloat(getComputedStyle(root).getPropertyValue('--sn-kb')) || 0;
      setHeight((prev) => {
        const next = rememberedHeight(prev, kb);
        if (next !== prev) {
          try {
            localStorage.setItem(KEY, String(next));
          } catch {
            /* storage off: the height lives for this session only */
          }
        }
        return next;
      });
    };
    read();
    const observer = new MutationObserver(read);
    observer.observe(root, { attributes: true, attributeFilter: ['style'] });
    return () => observer.disconnect();
  }, []);
  return height;
}
