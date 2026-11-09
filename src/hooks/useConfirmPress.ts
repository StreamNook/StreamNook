// A press that asks to be pressed again before it acts: the first press arms
// it, a second press within `windowMs` runs the action, and it disarms on its
// own when the window passes or the caller calls `disarm` (the pointer left).
// For the one-click actions that are hard to undo, like unfollowing a channel
// someone has followed for years (SN-264). The control keeps its size while
// armed, so nothing around it moves. A list of such controls (one per row)
// passes each row's key, so only the row pressed is armed.
import { useCallback, useEffect, useRef, useState } from 'react';

export const CONFIRM_WINDOW_MS = 3000;

/** An armed control's look: the same size, tinted red, so it reads "press
 *  again to do the destructive thing" without anything around it moving. */
export const ARMED_DANGER_CLASS = '!bg-error/25 ring-1 ring-inset ring-error/70';

export function useConfirmPress(windowMs = CONFIRM_WINDOW_MS) {
  const [armedKey, setArmedKey] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clear = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };

  const disarm = useCallback(() => {
    clear();
    setArmedKey(null);
  }, []);

  /** True when this press should act; false when it only armed `key`. */
  const press = useCallback(
    (key = ''): boolean => {
      clear();
      if (armedKey === key) {
        setArmedKey(null);
        return true;
      }
      setArmedKey(key);
      timer.current = setTimeout(() => {
        timer.current = null;
        setArmedKey(null);
      }, windowMs);
      return false;
    },
    [armedKey, windowMs],
  );

  useEffect(() => clear, []);

  return { armed: armedKey !== null, armedKey, press, disarm };
}
