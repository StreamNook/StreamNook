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

/** The same filled button for an action that can't be taken back (Ban):
 *  the theme's red mixed into black, kept dark enough for white text. */
export const DANGER_FILL: CSSProperties = { background: 'color-mix(in srgb, var(--color-error) 62%, black)' };

/** Every other button beside it (Deny, Cancel, a timeout length, a saved
 *  reason): the same capsule and `glaze-inset` lighting on a light film, a
 *  step quieter than the filled one. Callers set height, padding and size. */
export const GLAZE_PILL =
  'glaze-inset rounded-full bg-white/[0.08] font-semibold text-textPrimary transition-[background-color,filter] hover:bg-white/[0.14] active:brightness-95 disabled:opacity-50 disabled:hover:bg-white/[0.08]';

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
