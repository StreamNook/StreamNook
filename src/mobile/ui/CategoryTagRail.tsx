// The chip row under a category's title on the phone: a search chip that opens
// the tag sheet, then the screen's row of tags from Rust (new picks first, the
// category's own tags, then common tags of its live streams). The row only grows
// while the screen lives, so a chip never slides under a finger, and a picked
// chip changes color, never size.
import React from 'react';
import { MagnifyingGlass } from 'phosphor-react';
import { hasTag } from '../../utils/tagFilter';

export const CategoryTagRail: React.FC<{
  labels: readonly string[];
  selected: readonly string[];
  onToggle: (tag: string) => void;
  onSearch: () => void;
}> = ({ labels, selected, onToggle, onSearch }) => (
  <div className="shrink-0 flex items-center gap-2 overflow-x-auto scrollbar-none mask-edges px-4 pt-0.5 pb-2.5">
    <button
      type="button"
      onClick={onSearch}
      aria-label="Search tags"
      className="shrink-0 flex h-8 items-center gap-1.5 rounded-full bg-surface px-3 text-[13px] font-semibold text-textPrimary active:opacity-70"
    >
      <MagnifyingGlass size={15} weight="bold" />
      Tags
    </button>
    {labels.map((tag) => {
      const on = hasTag(selected, tag);
      return (
        <button
          key={tag}
          type="button"
          aria-pressed={on}
          onClick={() => onToggle(tag)}
          className={`shrink-0 flex h-8 items-center rounded-full px-3 text-[13px] font-semibold whitespace-nowrap transition-colors active:opacity-70 ${
            on ? 'bg-accent/20 text-accent' : 'bg-surface text-textSecondary'
          }`}
        >
          {tag}
        </button>
      );
    })}
  </div>
);
