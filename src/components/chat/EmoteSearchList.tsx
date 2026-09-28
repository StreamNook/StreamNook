import React, { useEffect, useRef } from 'react';
import { motion } from 'framer-motion';
import { GalleryHorizontal } from 'lucide-react';
import EmoteThumb from './EmoteThumb';
import EmoteProviderLogo, { type EmoteProviderId } from './EmoteProviderLogo';
import type { EmoteMatchRow } from '../../services/emoteMatch';
import { emoteOptionId } from '../../utils/chatInputWord';

interface EmoteSearchListProps {
  /** Listbox id; the textarea points at it with aria-controls. */
  id: string;
  /** What was typed; empty while browsing. */
  query: string;
  rows: EmoteMatchRow[];
  /** Matches in all; more than `rows.length` means the list is capped. */
  total: number;
  ready: boolean;
  selectedIndex: number;
  onSelect: (row: EmoteMatchRow) => void;
  onSelectedIndexChange: (index: number) => void;
  /** Make Tab open the carousel instead, from right here. Absent on a list
   *  opened with ":", which Tab's setting does not govern. */
  onSwitchToCarousel?: () => void;
}

// Browse headings that are a provider get its logo beside the title.
const GROUP_PROVIDER: Record<string, EmoteProviderId> = {
  Twitch: 'twitch',
  '7TV': '7tv',
  BTTV: 'bttv',
  FFZ: 'ffz',
  Kick: 'kick',
  YouTube: 'youtube',
};

const Kbd: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <kbd className="px-1 py-px rounded border border-white/10 bg-white/[0.06] font-sans text-[9px] leading-none text-white/60">
    {children}
  </kbd>
);

/** The name with the typed text picked out, where Rust could place it. */
const RowName: React.FC<{ row: EmoteMatchRow }> = ({ row }) => {
  const { name, matchAt, matchLen } = row;
  if (matchAt === undefined || !matchLen) {
    return <>{name}</>;
  }
  return (
    <>
      {name.slice(0, matchAt)}
      <span className="text-textPrimary font-semibold">{name.slice(matchAt, matchAt + matchLen)}</span>
      {name.slice(matchAt + matchLen)}
    </>
  );
};

/**
 * The emote list: every emote the viewer can use that matches what they typed,
 * with where each one comes from. Opened by `:` plus two letters or by Tab; the
 * rows arrive ranked from Rust.
 *
 * The popover itself never scrolls. `.sn-popover` draws its rim light as an
 * absolutely positioned layer, which scrolled away with the rows (and a sticky
 * header blurred over it) when the popover was the scroller, so only the body
 * scrolls here.
 */
const EmoteSearchList: React.FC<EmoteSearchListProps> = ({
  id,
  query,
  rows,
  total,
  ready,
  selectedIndex,
  onSelect,
  onSelectedIndexChange,
  onSwitchToCarousel,
}) => {
  const itemRefs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => {
    itemRefs.current[selectedIndex]?.scrollIntoView({ block: 'nearest' });
  }, [selectedIndex]);

  const capped = total > rows.length;
  const trimmed = query.replace(/^:/, '');

  return (
    <motion.div
      initial={{ opacity: 0, y: 6, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: 'easeOut' }}
      className="sn-popover absolute z-[60] w-full flex flex-col overflow-hidden origin-bottom"
      style={{ bottom: '100%', left: 0, right: 0, marginBottom: '8px' }}
      // Keeps the textarea focused (and the list open) when the scrollbar is used.
      onMouseDown={(e) => e.preventDefault()}
    >
      <div className="flex items-center gap-2 px-3 pt-2.5 pb-2 border-b border-white/[0.06]">
        <span className="text-[13px] font-semibold text-textPrimary">Emotes</span>
        {ready && rows.length > 0 && (
          <span className="px-1.5 py-0.5 rounded-full bg-white/[0.08] text-[10px] font-medium tabular-nums leading-none text-white/65">
            {capped ? `${rows.length} of ${total.toLocaleString()}` : total.toLocaleString()}
          </span>
        )}
        {onSwitchToCarousel && (
          <button
            type="button"
            title="Make Tab open the carousel instead"
            className="ml-auto shrink-0 inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] leading-none text-white/60 hover:text-textPrimary hover:bg-white/[0.07] transition-colors"
            // mousedown, not click: the textarea keeps focus and its caret.
            onMouseDown={(e) => {
              e.preventDefault();
              onSwitchToCarousel();
            }}
          >
            <GalleryHorizontal className="w-3.5 h-3.5" />
            Carousel
          </button>
        )}
      </div>

      {!ready && rows.length === 0 ? (
        <div className="px-3 py-3 text-xs text-textSecondary">Emotes are still loading</div>
      ) : rows.length === 0 ? (
        <div className="px-3 py-3 text-xs text-textSecondary">
          {trimmed ? <>No emotes match &ldquo;{trimmed}&rdquo;</> : 'No emotes here yet'}
        </div>
      ) : (
        <div
          id={id}
          role="listbox"
          aria-label="Emotes"
          className="overflow-y-auto custom-scrollbar max-h-[min(400px,50vh)] p-1"
        >
          {rows.map((row, index) => {
            const heading = row.group && row.group !== rows[index - 1]?.group ? row.group : null;
            const headingProvider = heading ? GROUP_PROVIDER[heading] : undefined;
            const selected = index === selectedIndex;
            return (
              <React.Fragment key={`${row.provider}-${row.id}-${row.name}`}>
                {heading && (
                  <div className="flex items-center gap-1.5 px-2 pt-2.5 pb-1 text-[10px] font-semibold uppercase tracking-wide text-white/45">
                    {headingProvider && <EmoteProviderLogo provider={headingProvider} tinted className="w-3 h-3" />}
                    {heading}
                  </div>
                )}
                <div
                  ref={(el) => {
                    itemRefs.current[index] = el;
                  }}
                  id={emoteOptionId(id, index)}
                  role="option"
                  aria-selected={selected}
                  className={`flex items-center gap-2.5 px-2 py-1 rounded-md cursor-pointer transition-colors ${
                    selected ? 'bg-accent/15 ring-1 ring-inset ring-accent/30' : 'hover:bg-white/[0.05]'
                  }`}
                  // mousedown, not click: the textarea keeps focus and its caret.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    onSelect(row);
                  }}
                  onMouseEnter={() => onSelectedIndexChange(index)}
                >
                  <span className="w-8 h-8 shrink-0 rounded-md bg-white/[0.04] flex items-center justify-center">
                    <EmoteThumb emote={row} size={26} />
                  </span>
                  <span className="flex-1 min-w-0 truncate text-[13px] text-textPrimary/80">
                    <RowName row={row} />
                  </span>
                  <span className="shrink-0 inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/[0.05] text-[10px] leading-none text-white/55">
                    <EmoteProviderLogo provider={row.provider} tinted className="w-3 h-3" />
                    {row.sourceDetail}
                  </span>
                </div>
              </React.Fragment>
            );
          })}
          {capped && (
            <div className="px-2 pt-2 pb-1 text-[10px] text-white/35">Keep typing to narrow the list</div>
          )}
        </div>
      )}

      {rows.length > 0 && (
        <div className="flex items-center gap-1 px-3 py-1.5 border-t border-white/[0.06] whitespace-nowrap overflow-hidden text-[10px] text-white/40">
          <Kbd>↑↓</Kbd>
          <span className="mr-1.5">move</span>
          <Kbd>Enter</Kbd>
          <span className="mr-1.5">insert</span>
          <Kbd>Esc</Kbd>
          <span>close</span>
        </div>
      )}
    </motion.div>
  );
};

export default EmoteSearchList;
