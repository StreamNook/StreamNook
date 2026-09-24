import type { CSSProperties } from 'react';

// The glazed action buttons that sit inside a glass surface (a popover, the
// changelog popup): `glaze-inset` lighting, no frost of their own.

/** The one filled button on a surface. Pair with `ACCENT_FILL`; callers set
 *  padding and text size. */
export const ACCENT_BUTTON =
  'glaze-inset rounded-full text-[13.5px] font-semibold text-white transition-[filter] hover:brightness-110 active:brightness-95 disabled:opacity-60 disabled:hover:brightness-100';

/** Accent mixed into black, so it stays dark enough for white text on every
 *  theme, including the ones whose accent is pale. */
export const ACCENT_FILL: CSSProperties = { background: 'color-mix(in srgb, var(--color-accent) 55%, black)' };
