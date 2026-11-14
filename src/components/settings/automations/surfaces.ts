// The two card surfaces of the automations page, per platform.
import { IS_MOBILE } from '../../../utils/platform';

/** A group's card: the settings slab on the desktop, the phone's solid group card. */
export const CARD = IS_MOBILE ? 'sn-group-card' : 'settings-card';

/** The one card that steps above the rest: what is running right now. An
 *  accent tint over the card's own surface. */
export const RAISED_CARD = IS_MOBILE
  ? 'overflow-hidden rounded-[14px] [background:linear-gradient(color-mix(in_srgb,var(--color-accent)_13%,transparent),color-mix(in_srgb,var(--color-accent)_13%,transparent)),var(--color-surface)]'
  : 'overflow-hidden rounded-[10px] bg-accent/[0.10]';
