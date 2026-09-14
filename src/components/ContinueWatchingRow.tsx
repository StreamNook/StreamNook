import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, Clock, X } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { MediaCard } from './MediaCard';
import { formatRemaining, formatVodTime, vodProgressLabel, vodThumbUrl } from '../utils/vodProgress';
import type { ContinueWatchingItem } from '../types';

/**
 * Home's "pick up where you left off" rail.
 *
 * Rust owns the list (`home_snapshot`'s `continue_watching` section, derived
 * from the local VOD watch-position store), so this component fetches nothing
 * and paints on the first frame after a cold start. It only ever shows VODs
 * the viewer deliberately opened: live sessions, live rewinds and the offline
 * auto-play all record no position (see `isDeliberateVod` in VideoPlayer).
 *
 * A rail rather than a grid so an unfinished VOD never pushes live channels
 * down the page.
 */

/** Sized to the live grid cards below it: a narrower rail card reads as a
 *  lesser element no matter how it is styled. */
const CARD_WIDTH = 300;
/** Card width plus the flex gap, for one chevron click. */
const CARD_STRIDE = CARD_WIDTH + 12;

export default function ContinueWatchingRow() {
  const items = useAppStore((s) => s.continueWatching);
  const railRef = useRef<HTMLDivElement>(null);
  const [overflowing, setOverflowing] = useState(false);

  // Chevrons only exist when there is somewhere to scroll. Measured rather
  // than guessed from the count, because the rail's width is the variable.
  useEffect(() => {
    const rail = railRef.current;
    if (!rail) return;
    const measure = () => setOverflowing(rail.scrollWidth > rail.clientWidth + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(rail);
    return () => observer.disconnect();
  }, [items.length]);

  const scrollBy = useCallback((direction: -1 | 1) => {
    railRef.current?.scrollBy({ left: direction * CARD_STRIDE, behavior: 'smooth' });
  }, []);

  // Home's pattern: an empty section renders nothing at all, chrome included.
  if (items.length === 0) return null;

  return (
    <div className="mb-6">
      <div className="pb-3 px-2 flex justify-between items-center">
        <h3 className="text-sm font-semibold text-textSecondary uppercase tracking-wide flex items-center gap-2">
          <Clock size={14} className="text-textSecondary/70" />
          Continue Watching
        </h3>
      </div>

      <div className="relative group">
        <div ref={railRef} className="sn-rail flex gap-3 overflow-x-auto snap-x px-2 pb-1">
          {items.map((item) => (
            <ContinueWatchingCard key={item.video_id} item={item} />
          ))}
        </div>

        {overflowing && (
          <>
            <button
              type="button"
              aria-label="Scroll left"
              onClick={() => scrollBy(-1)}
              className="glass-button absolute left-0 top-1/2 -translate-y-1/2 z-10 rounded-full p-1.5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
            >
              <ChevronLeft size={16} />
            </button>
            <button
              type="button"
              aria-label="Scroll right"
              onClick={() => scrollBy(1)}
              className="glass-button absolute right-0 top-1/2 -translate-y-1/2 z-10 rounded-full p-1.5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
            >
              <ChevronRight size={16} />
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/** One resumable VOD: the shared media card with time LEFT in the stat slot
 *  (what a viewer deciding whether to resume actually weighs) and a hover-only
 *  dismiss in the corner. */
function ContinueWatchingCard({ item }: { item: ContinueWatchingItem }) {
  const playMedia = useAppStore((s) => s.playMedia);
  const dismiss = useAppStore((s) => s.dismissContinueWatching);
  const setProfileModalUser = useAppStore((s) => s.setProfileModalUser);

  const open = () => {
    setProfileModalUser(null);
    void playMedia('video', `https://www.twitch.tv/videos/${item.video_id}`, {
      id: item.video_id,
      // `user_login` is load-bearing: it binds the channel for VOD chat replay.
      user_login: item.channel_login,
      user_name: item.channel_name,
      title: item.title,
      thumbnail_url: item.thumbnail_url,
    });
  };

  const progress = {
    position_secs: item.position_secs,
    duration_secs: item.duration_secs,
    completed: false,
  };
  const remaining = formatRemaining(item.position_secs, item.duration_secs);
  const resumeLabel = vodProgressLabel(progress);
  const title = item.title || 'Past broadcast';

  return (
    <MediaCard
      kind="vod"
      title={title}
      thumbnailUrl={vodThumbUrl(item.thumbnail_url)}
      durationLabel={item.duration_secs > 0 ? formatVodTime(item.duration_secs) : undefined}
      stat={remaining ?? resumeLabel ?? undefined}
      progress={progress}
      lengthSeconds={item.duration_secs}
      channel={{
        name: item.channel_name,
        avatarUrl: item.profile_image_url || undefined,
        partner: item.partner,
      }}
      category={item.game_name || undefined}
      topRight={
        <button
          type="button"
          aria-label={`Remove ${title} from Continue Watching`}
          onClick={(e) => {
            // The whole card is the play target.
            e.stopPropagation();
            void dismiss(item.video_id);
          }}
          className="glass-button absolute top-2 right-2 rounded-full p-1.5 text-white/90 opacity-0 transition-opacity group-hover/card:opacity-100 focus-visible:opacity-100"
        >
          <X size={12} />
        </button>
      }
      onClick={open}
      className="shrink-0 snap-start"
      style={{ width: CARD_WIDTH }}
      ariaLabel={resumeLabel ? `${title}. ${resumeLabel}` : title}
    />
  );
}
