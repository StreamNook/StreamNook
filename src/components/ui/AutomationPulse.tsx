import type { ReactElement } from 'react';

export type AutomationTone = 'silver' | 'gold' | 'iridescent';

/** The drops-button glow while automation runs: silver for channel points,
 *  gold for drops, iridescent for both.
 *
 *  Only `opacity` animates, on an HTML wrapper, so the pulse runs on the
 *  compositor. Animating the icon's colour or its drop-shadow repaints on the
 *  main thread every frame, and in a window this size every repaint re-layers
 *  the whole page: that one icon held the main thread near 100% and made the
 *  startup animations stutter. The iridescent shift is two tinted copies of the
 *  icon crossfading, which keeps it opacity-only too. */
export const AutomationPulse = ({ tone, children }: { tone: AutomationTone; children: ReactElement }) => (
  <span className={`automation-pulse automation-pulse--${tone}`}>
    {children}
    {tone === 'iridescent' && (
      <span aria-hidden className="automation-pulse-shift">
        {children}
      </span>
    )}
  </span>
);
