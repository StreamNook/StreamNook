// A scrolling grid that adapts to the window shape, and to a hinge if one
// crosses the screen.
//
// Two things it does that a set of Tailwind breakpoints cannot:
//
//  1. Tailwind's sm/md/lg are viewport-width buckets, so on a 840dp unfolded
//     Fold a `md:grid-cols-2` puts its gutter at 50% of the viewport. That is
//     the one place it must not go: the crease. This aligns the gutter TO the
//     seam instead, so the fold falls between two cards rather than through
//     one.
//  2. The right column count depends on what is being laid out. A 16:9 card
//     wants a different count than a short row, and a phone-width column of
//     giant cards on a tablet shows exactly one stream, which is fewer than
//     the phone manages.
import React from 'react';
import { useWindowShape } from './useWindowShape';

interface Props {
  /** `card` is the 16:9 poster; `row` is the short horizontal item. */
  variant: 'card' | 'row';
  /** Horizontal padding of the scroll container, in px. Used for seam maths. */
  padX?: number;
  gap?: number;
  className?: string;
  children: React.ReactNode;
}

/** Column count per size class. Rows are wider, so they get fewer. */
function columnsFor(variant: 'card' | 'row', sizeClass: string, landscape: boolean): number {
  if (sizeClass === 'compact') {
    // Phones stay single column. A 2-up card grid at 360dp is unreadable.
    return variant === 'row' && landscape ? 2 : 1;
  }
  if (sizeClass === 'medium') return 2;
  return variant === 'card' ? 3 : 2;
}

export const AdaptiveGrid: React.FC<Props> = ({
  variant,
  padX = 16,
  gap = 12,
  className = '',
  children,
}) => {
  const shape = useWindowShape();
  const cols = columnsFor(variant, shape.sizeClass, shape.landscape);

  // A vertical hinge only dictates the layout when the split is two-up: with
  // three columns there is no single gutter to give the seam, and forcing one
  // would leave a lopsided grid for no gain.
  const seam =
    cols === 2 && shape.fold?.vertical && shape.fold.x > padX && shape.fold.x < shape.w - padX
      ? shape.fold
      : null;

  const style: React.CSSProperties = seam
    ? {
        display: 'grid',
        // Left column runs from the content edge to the hinge; the gutter IS
        // the hinge, so nothing is ever rendered on the crease.
        gridTemplateColumns: `${seam.x - padX}px 1fr`,
        columnGap: `${Math.max(seam.width, gap)}px`,
        rowGap: `${gap}px`,
      }
    : {
        display: 'grid',
        gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
        gap: `${gap}px`,
      };

  return (
    <div className={className} style={style}>
      {children}
    </div>
  );
};
