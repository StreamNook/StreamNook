import { useEffect, type ReactNode } from 'react';
import { MotionConfig } from 'framer-motion';
import { useAppStore } from '../stores/AppStore';
import type { MotionMode } from '../types';
import { useLowPower } from '../utils/lowPower';

/**
 * Applies the user's Motion preference (Interface > Motion) across the whole
 * window. Two layers, one source of truth:
 *
 *  - `data-motion` on <html> drives the CSS layer. globals.css gates its
 *    transition/animation overrides on it ('off' collapses everything to
 *    instant; 'reduced' strips the bigger movements and the button bounce).
 *  - framer-motion's `reducedMotion` handles the JS-driven animations. 'full'
 *    keeps them; 'reduced' and 'off' drop transform/layout motion (fades stay).
 *
 * Defaults to 'full' when unset, so existing users see no change until they opt
 * in. Reading from the store keeps it reactive: flipping the setting re-applies
 * immediately without a reload.
 */
export function MotionScope({ children }: { children: ReactNode }) {
  const chosen: MotionMode = useAppStore((s) => s.settings.motion_mode) ?? 'full';
  // Battery Saver (phone only, utils/lowPower) steps Full down to Reduced: the
  // decorative loops stop, quick fades stay. The setting itself is untouched.
  const lowPower = useLowPower();
  const mode: MotionMode = lowPower && chosen === 'full' ? 'reduced' : chosen;

  useEffect(() => {
    document.documentElement.dataset.motion = mode;
  }, [mode]);

  return (
    <MotionConfig reducedMotion={mode === 'full' ? 'never' : 'always'}>
      {children}
    </MotionConfig>
  );
}
