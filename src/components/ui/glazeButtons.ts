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

/** A banner in the chat column (the watch-streak and resub shares, over the
 *  composer): the chat header capsules' glaze and accent-lit rim. It keeps the
 *  glaze's blur, since what sits behind it is the column's own ground or a
 *  theme's art, not repainting rows. Solid at Glassiness 0, like the glaze.
 *  Pair with `BANNER_STYLE`. */
export const GLAZE_BANNER = 'chrome-glaze';

/** A card's corner (the glaze's own rule rounds to a pill and outranks the
 *  rounded utilities, so it is set inline), and a little frost, so the banner's
 *  quieter lines stay readable over a theme's art. */
export const BANNER_STYLE = { borderRadius: 14, '--glaze-frost': '24%' } as CSSProperties;
