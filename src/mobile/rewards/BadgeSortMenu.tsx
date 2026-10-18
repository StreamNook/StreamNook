// The badge wall's sort: a pill naming the current one, opening a menu of all
// five. The capsule it replaced scrolled sideways inside its glass, so on a
// phone "Oldest" sat off screen where nobody found it.
import { useRef, useState } from 'react';
import { CaretDown, Check } from 'phosphor-react';
import { RewardsPopover } from './RewardsPopover';
import type { BadgeSort } from './rewardsModel';

const BADGE_SORTS: { id: BadgeSort; label: string }[] = [
  { id: 'newest', label: 'Newest' },
  { id: 'available', label: 'Available' },
  { id: 'soon', label: 'Coming soon' },
  { id: 'usage', label: 'Most used' },
  { id: 'oldest', label: 'Oldest' },
];

export function BadgeSortMenu({ sort, onChange }: { sort: BadgeSort; onChange: (sort: BadgeSort) => void }) {
  const [open, setOpen] = useState(false);
  const pill = useRef<HTMLButtonElement>(null);
  const current = BADGE_SORTS.find((s) => s.id === sort) ?? BADGE_SORTS[0];

  return (
    <>
      {/* An accessory to the heading, so a step quieter than it: secondary
          text on the plain glaze, no accent. */}
      <button
        ref={pill}
        type="button"
        onClick={() => setOpen(true)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Sort badges, ${current.label}`}
        className="chrome-glaze chrome-glaze--flat shrink-0 h-[30px] px-3 inline-flex items-center gap-1.5 text-[12.5px] font-semibold text-textSecondary active:opacity-70 transition-opacity"
      >
        <span>{current.label}</span>
        <CaretDown size={10} weight="bold" aria-hidden />
      </button>
      <RewardsPopover
        open={open}
        onClose={() => setOpen(false)}
        anchor={pill}
        width={200}
        align="end"
        role="menu"
        label="Sort badges"
      >
        {BADGE_SORTS.map((s) => {
          const chosen = s.id === sort;
          return (
            <button
              key={s.id}
              type="button"
              role="menuitemradio"
              aria-checked={chosen}
              onClick={() => {
                setOpen(false);
                if (!chosen) onChange(s.id);
              }}
              className="sn-menu-row"
            >
              <span className="flex-1 truncate">{s.label}</span>
              {/* The slot stays when empty, so every label keeps one width. */}
              <span className="flex w-4 shrink-0 items-center justify-center text-accent">
                {chosen && <Check size={16} weight="bold" aria-hidden />}
              </span>
            </button>
          );
        })}
      </RewardsPopover>
    </>
  );
}
