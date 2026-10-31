// A card's tag line on a category screen: the stream's tags in one line that
// scrolls sideways, each a pill that adds or removes it from the screen's
// filter. It is laid over a band the card reserves, never inside the card's
// button (no nested buttons), and the band carries the card's own press
// handlers, so a tap or hold beside a pill acts as the card. Nothing here is
// measured and no store is read: a category screen holds up to 320 of these.
import React from 'react';
import { hasTag, pillClick } from '../../utils/tagFilter';

/** What a category screen hands its cards so they can draw and toggle tags. */
export interface CardTagFilter {
  selected: readonly string[];
  onToggle: (tag: string) => void;
}

export const StreamCardTags: React.FC<{
  tags: readonly string[];
  filter: CardTagFilter;
  /** The card's own press handlers (tap watches, hold opens its sheet). */
  bandHandlers: React.DOMAttributes<HTMLDivElement>;
  /** Reads and clears "that press was a hold", so a pill does nothing then. */
  consumeHeld: () => boolean;
}> = ({ tags, filter, bandHandlers, consumeHeld }) => (
  <div
    {...bandHandlers}
    role="group"
    aria-label="Tags"
    className="sn-tag-line absolute inset-x-0 bottom-0 h-[42px] select-none [-webkit-touch-callout:none]"
  >
    <div className="sn-tag-track flex h-full items-start gap-1.5 overflow-x-auto px-2.5 [scrollbar-width:none]">
      {tags.map((tag) => {
        const on = hasTag(filter.selected, tag);
        return (
          <button
            key={tag}
            type="button"
            data-tag-chip=""
            aria-pressed={on}
            // The whole band's height is the pill's touch target; the visible
            // pill is 28 px at its top.
            className="h-full shrink-0 pt-1"
            onClick={(e) =>
              pillClick({ stop: () => e.stopPropagation(), consumeHeld, toggle: () => filter.onToggle(tag) })
            }
          >
            <span
              className={`flex h-7 items-center rounded-full px-2.5 text-[12px] font-semibold leading-none whitespace-nowrap transition-colors ${
                on ? 'bg-accent/20 text-accent' : 'bg-surface text-textSecondary'
              }`}
            >
              {tag}
            </span>
          </button>
        );
      })}
    </div>
  </div>
);
