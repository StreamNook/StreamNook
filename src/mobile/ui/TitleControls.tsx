// Controls in a title row, grouped the way the iPhone groups them: each cluster
// in a 44px glass capsule, lighter than the title it serves. Bare icons beside
// a large title read as stray marks, and two of them side by side measured
// 1.6:1 against the page.
import React, { useId } from 'react';
import { motion } from 'framer-motion';
import { ListBullets, SquaresFour } from 'phosphor-react';
import { TitleMenu, type TitleMenuOption } from './PlatformMenu';

/** A 44px glass capsule holding one cluster of title-row controls. */
export const TitleCapsule: React.FC<{ children: React.ReactNode; className?: string }> = ({ children, className = '' }) => (
  <div className={`chrome-glaze flex h-11 items-center px-1 ${className}`}>{children}</div>
);

/**
 * A screen's parts as a compact two- or three-part capsule in the title row
 * ("Drops | Badges"), so choosing between them spends no row of its own. The
 * chosen part sits in a darker pill that glides; the others are secondary
 * text. Each label keeps its bold width underneath, so the capsule never
 * changes width when the choice moves.
 */
export function TitleSegments<T extends string>({
  segments,
  value,
  onChange,
  label,
}: {
  segments: { value: T; label: string }[];
  value: T;
  onChange: (next: T) => void;
  /** What the group chooses, for screen readers. */
  label: string;
}) {
  const pillId = useId();
  return (
    <TitleCapsule>
      <div role="radiogroup" aria-label={label} className="flex items-center">
        {segments.map((s) => {
          const on = s.value === value;
          return (
            <button
              key={s.value}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => {
                if (!on) onChange(s.value);
              }}
              className={`relative h-9 rounded-full px-3 text-[14px] leading-none transition-colors ${
                on ? 'text-textPrimary' : 'text-textSecondary'
              }`}
            >
              {on && (
                <motion.span
                  layoutId={`title-segment-pill-${pillId}`}
                  className="absolute inset-0 glaze-selected"
                  transition={{ type: 'spring', stiffness: 350, damping: 30 }}
                />
              )}
              <span className="relative z-10 grid">
                <span aria-hidden className="invisible col-start-1 row-start-1 font-semibold">
                  {s.label}
                </span>
                <span className={`col-start-1 row-start-1 text-center ${on ? 'font-semibold' : ''}`}>{s.label}</span>
              </span>
            </button>
          );
        })}
      </div>
    </TitleCapsule>
  );
}

export type StreamView = 'cards' | 'list';

const VIEW_OPTIONS: TitleMenuOption<StreamView>[] = [
  { value: 'cards', label: 'Cards', icon: <SquaresFour size={20} /> },
  { value: 'list', label: 'List', icon: <ListBullets size={20} /> },
];

/**
 * Cards or a list, as one title-row menu (the platform menu's): the layout in
 * use with a small chevron, and the choice hung from it. One capsule instead
 * of a two-part switch, so the title row stays uncluttered beside the inbox.
 */
export const ViewMenu: React.FC<{ value: StreamView; onChange: (next: StreamView) => void }> = ({ value, onChange }) => (
  <TitleCapsule>
    <TitleMenu label="Layout" options={VIEW_OPTIONS} value={value} onChange={onChange} />
  </TitleCapsule>
);
