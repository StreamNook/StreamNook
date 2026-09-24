import { forwardRef } from 'react';
import { motion } from 'framer-motion';
import { Clapperboard } from 'lucide-react';
import type { VodChapter } from '../types';
import { formatVodTime } from '../utils/vodProgress';

/**
 * The chapter list for a VOD: every category the broadcast ran under, with
 * its box art, start time and length. Opened from the Chapters control in the
 * player bar; a row seeks the player to that chapter.
 *
 * An opaque plate, not glass: menus over video are opaque on purpose so
 * their rows stay legible whatever plays underneath.
 */
const VodChaptersMenu = forwardRef<
  HTMLDivElement,
  {
    chapters: VodChapter[];
    current: VodChapter | null;
    onSeek: (secs: number) => void;
  }
>(function VodChaptersMenu({ chapters, current, onSeek }, ref) {
  return (
    <motion.div
      ref={ref}
      initial={{ opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 8, scale: 0.98 }}
      transition={{ duration: 0.16, ease: 'easeOut' }}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
      data-no-wheel-volume
      role="menu"
      aria-label="Chapters"
      className="stats-hud absolute bottom-16 left-3 z-[60] w-80 max-h-[60%] overflow-y-auto scrollbar-thin rounded-xl p-2"
    >
      <div className="px-2 pb-1.5 pt-1 text-[11px] font-semibold uppercase tracking-wider text-textSecondary">
        Chapters
      </div>
      {chapters.map((c) => {
        const active = current?.start_secs === c.start_secs;
        return (
          <button
            key={c.start_secs}
            type="button"
            role="menuitem"
            onClick={() => onSeek(c.start_secs)}
            className={`flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors ${
              active ? 'bg-accent/15' : 'hover:bg-white/10'
            }`}
          >
            {c.box_art_url ? (
              <img
                src={c.box_art_url}
                alt=""
                width={36}
                height={48}
                loading="lazy"
                className="h-12 w-9 shrink-0 rounded object-cover"
              />
            ) : (
              <div className="flex h-12 w-9 shrink-0 items-center justify-center rounded bg-white/5">
                <Clapperboard size={14} className="text-textSecondary" />
              </div>
            )}
            <div className="min-w-0 flex-1">
              <div className={`truncate text-[13px] font-medium ${active ? 'text-accent' : 'text-textPrimary'}`}>
                {c.title}
              </div>
              <div className="text-[12px] tabular-nums text-textSecondary">
                {formatVodTime(c.start_secs)} · {formatVodTime(c.end_secs - c.start_secs)}
              </div>
            </div>
          </button>
        );
      })}
    </motion.div>
  );
});

export default VodChaptersMenu;
