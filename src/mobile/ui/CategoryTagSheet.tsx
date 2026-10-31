// The tag search on the phone, in the platform sheet. Rust ranks the matches
// (prefix first); "Filter by tag" offers the typed text whenever no match is it,
// since Twitch filters by any tag name. A pick toggles the tag and clears the
// field; the sheet and the keyboard stay for the next one. The sheet portals
// outside .sn-mobile, so only global classes are used here.
import React, { useState } from 'react';
import { Check, MagnifyingGlass, Plus } from 'phosphor-react';
import { MobileSheet } from './MobileSheet';
import { useCategoryTagSuggestions } from '../../hooks/useCategoryTagSuggestions';
import { hasTag } from '../../utils/tagFilter';

export const CategoryTagSheet: React.FC<{
  open: boolean;
  onClose: () => void;
  gameName: string;
  selected: readonly string[];
  onToggle: (tag: string) => void;
  revision: unknown;
}> = ({ open, onClose, gameName, selected, onToggle, revision }) => {
  const [query, setQuery] = useState('');
  const answer = useCategoryTagSuggestions({ gameName, query, selected, limit: 14, enabled: open, revision });
  const labels = answer?.labels ?? [];
  const typed = answer?.offer_typed ?? null;

  const pick = (tag: string) => {
    onToggle(tag);
    setQuery('');
  };

  return (
    <MobileSheet
      open={open}
      onClose={() => {
        setQuery('');
        onClose();
      }}
      title="Tags"
      fill
      maxHeightFraction={0.85}
    >
      <div className="flex flex-col gap-3 px-3.5 pb-3.5">
        <div className="relative">
          <MagnifyingGlass size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-textMuted z-10" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== 'Enter') return;
              const q = query.trim();
              if (!q) return;
              e.preventDefault();
              // The best match Rust gave for exactly this text, else the text.
              pick(labels[0] ?? q);
            }}
            enterKeyHint="search"
            placeholder="Search tags"
            aria-label="Search tags"
            className="glass-input w-full pl-9 pr-3 py-2.5 text-[14px] text-textPrimary"
          />
        </div>

        {/* A press here picks and keeps the keyboard up for the next tag. */}
        <div data-keeps-keyboard="" className="flex flex-col gap-3">
          {typed && (
            <button
              type="button"
              onClick={() => pick(typed)}
              className="glass-panel flex items-center gap-3 p-3 text-left text-[14px] text-textPrimary active:opacity-70"
            >
              <Plus size={18} className="shrink-0 text-textSecondary" />
              <span className="min-w-0 truncate">
                Filter by tag <span className="font-semibold">"{typed}"</span>
              </span>
            </button>
          )}
          {labels.length > 0 ? (
            <div className="glass-panel divide-y divide-borderSubtle">
              {labels.map((tag) => {
                const on = hasTag(selected, tag);
                return (
                  <button
                    key={tag}
                    type="button"
                    aria-pressed={on}
                    onClick={() => pick(tag)}
                    className="flex w-full items-center gap-3 p-3 text-left text-[14px] active:opacity-70"
                  >
                    <span className={`min-w-0 flex-1 truncate ${on ? 'font-semibold text-accent' : 'text-textPrimary'}`}>
                      {tag}
                    </span>
                    {on && <Check size={18} weight="bold" className="shrink-0 text-accent" />}
                  </button>
                );
              })}
            </div>
          ) : (
            !typed && <p className="py-6 text-center text-[13px] text-textMuted">Type any tag to filter by it.</p>
          )}
        </div>
      </div>
    </MobileSheet>
  );
};
