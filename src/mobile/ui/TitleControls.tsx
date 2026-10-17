// Controls in a title row, grouped the way the iPhone groups them: each cluster
// in a 44px glass capsule, lighter than the title it serves. Bare icons beside
// a large title read as stray marks, and two of them side by side measured
// 1.6:1 against the page.
import React, { useId } from 'react';
import { motion } from 'framer-motion';
import { ListBullets, SquaresFour } from 'phosphor-react';

/** A 44px glass capsule holding one cluster of title-row controls. */
export const TitleCapsule: React.FC<{ children: React.ReactNode; className?: string }> = ({ children, className = '' }) => (
  <div className={`chrome-glaze flex h-11 items-center px-1 ${className}`}>{children}</div>
);

export type StreamView = 'cards' | 'list';

/**
 * Cards or a list, as a two-segment capsule: the chosen view sits in a darker
 * pill that glides between the two, the other is secondary text. The icons are
 * 20px regular outlines, one weight for the pair; the chosen one is filled.
 */
export const ViewToggle: React.FC<{ value: StreamView; onChange: (next: StreamView) => void }> = ({
  value,
  onChange,
}) => {
  // One pill per toggle: tab screens stay mounted, and two toggles sharing a
  // layoutId would trade pills.
  const pillId = useId();
  const choice = (view: StreamView, label: string, icon: React.ReactNode) => {
    const on = value === view;
    return (
      <button
        type="button"
        role="radio"
        aria-checked={on}
        aria-label={label}
        onClick={() => {
          if (!on) onChange(view);
        }}
        className={`relative flex h-9 w-10 items-center justify-center rounded-full transition-colors ${
          on ? 'text-textPrimary' : 'text-textSecondary'
        }`}
      >
        {on && (
          <motion.span
            layoutId={`title-view-pill-${pillId}`}
            className="absolute inset-0 glaze-selected"
            transition={{ type: 'spring', stiffness: 350, damping: 30 }}
          />
        )}
        <span className="relative z-10 flex">{icon}</span>
      </button>
    );
  };
  return (
    <TitleCapsule>
      <div role="radiogroup" aria-label="View" className="flex items-center">
        {choice('cards', 'Card view', <SquaresFour size={20} weight={value === 'cards' ? 'fill' : 'regular'} />)}
        {choice('list', 'List view', <ListBullets size={20} weight={value === 'list' ? 'bold' : 'regular'} />)}
      </div>
    </TitleCapsule>
  );
};
