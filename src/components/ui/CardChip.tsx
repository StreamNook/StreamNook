import type { HTMLAttributes, ReactNode } from 'react';

export type CardChipKind =
  | 'live'
  | 'drops'
  | 'hype'
  | 'hype-golden'
  | 'streak'
  | 'ready'
  | 'done'
  | 'neutral';

// The kinds that carry the shine sweep: few per grid, so the animation is
// affordable there. LIVE sits on every card and pulses its dot on hover instead.
const SHINES: ReadonlySet<CardChipKind> = new Set(['drops', 'hype', 'hype-golden']);

/** A status chip on a stream or category card (LIVE, DROPS, hype train,
 *  viewers, watch streak, a drop READY or DONE) in the title bar's material:
 *  the same frosted `.chrome-glaze` capsule, lit in the chip's own colour. The
 *  LIVE dot and the shine sweep are child elements because the glaze owns both
 *  pseudo-elements (its rim and its specular). Other props (a Tooltip's
 *  handlers, a style) reach the element. */
export const CardChip = ({
  kind,
  size = 'sm',
  flat = false,
  className = '',
  children,
  ...rest
}: {
  kind: CardChipKind;
  size?: 'sm' | 'lg';
  /** No frost: for a chip repeated per card on the phone (see `.card-chip--flat`). */
  flat?: boolean;
  className?: string;
  children: ReactNode;
} & Omit<HTMLAttributes<HTMLSpanElement>, 'children' | 'className'>) => (
  <span
    {...rest}
    className={`chrome-glaze chrome-glaze--frosted card-chip card-chip--${kind}${size === 'lg' ? ' card-chip--lg' : ''}${
      flat ? ' card-chip--flat' : ''
    } ${className}`}
  >
    {SHINES.has(kind) && (
      <span aria-hidden className="card-chip-shine">
        <span />
      </span>
    )}
    {kind === 'live' && <span aria-hidden className="card-chip-dot" />}
    {children}
  </span>
);
