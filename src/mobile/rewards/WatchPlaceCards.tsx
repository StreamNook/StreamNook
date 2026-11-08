// Where a badge or a Drops campaign can be earned, as tappable cards on the
// phones: its category (opens Browse at it) and its channels with their face
// and live line (opens the stream). Rust resolved every card
// (services/watch_places.rs); these only draw them, like desktop's badge page.
import { CaretRight, GameController } from 'phosphor-react';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import type { PlaceCategory, PlaceChannel } from '../../services/watchPlaces';

const ROW = 'glass-panel sn-touch w-full px-3 py-2.5 flex items-center gap-2.5 text-left active:opacity-80';

/** The action word and its chevron, one quiet group at the row's end. */
function Trailing({ word }: { word: string }) {
  return (
    <span className="flex items-center gap-1.5 shrink-0 text-textMuted">
      <span className="text-[12px]">{word}</span>
      <CaretRight size={13} weight="bold" />
    </span>
  );
}

/** Rust's "Live · Game · 1.2K viewers" with the count kept whole: only the
 *  part before it gives way on a narrow phone. A line with no count is one
 *  piece. */
const COUNT_TAIL = /^(.*) · (\S+ viewers?)$/;

export function PlaceCategoryCard({ place, onOpen }: { place: PlaceCategory; onOpen: () => void }) {
  return (
    <button onClick={onOpen} className={ROW}>
      {place.box_art_url ? (
        <img
          src={place.box_art_url}
          alt=""
          draggable={false}
          className="w-9 aspect-[3/4] object-cover rounded shrink-0"
        />
      ) : (
        <span className="w-9 aspect-[3/4] rounded bg-surface flex items-center justify-center shrink-0">
          <GameController size={16} className="text-textMuted" />
        </span>
      )}
      <div className="flex-1 min-w-0">
        <div className="text-[13.5px] font-medium text-textPrimary truncate">{place.name}</div>
        <div className="text-[12px] text-textMuted truncate">{place.viewers_line ?? 'Category'}</div>
      </div>
      <Trailing word="Browse" />
    </button>
  );
}

export function PlaceChannelCard({ place, onWatch }: { place: PlaceChannel; onWatch: () => void }) {
  const split = COUNT_TAIL.exec(place.status_line);
  return (
    <button onClick={onWatch} aria-label={`Watch ${place.display_name}`} className={ROW}>
      {place.avatar_url ? (
        <SevenTvAvatar
          cosmeticsId={place.user_id ?? undefined}
          px={36}
          src={place.avatar_url}
          alt=""
          className="w-9 h-9 rounded-full object-cover shrink-0"
          loading="lazy"
        />
      ) : (
        <div className="w-9 h-9 rounded-full bg-surface shrink-0" />
      )}
      <div className="flex-1 min-w-0">
        <div className="text-[13.5px] font-medium text-textPrimary truncate">{place.display_name}</div>
        <div className={`text-[12px] flex min-w-0 ${place.live ? 'text-red-400' : 'text-textMuted'}`}>
          <span className="truncate">{split ? split[1] : place.status_line}</span>
          {/* A flex item drops a leading space, so it is a no-break one. */}
          {split && <span className="shrink-0 whitespace-nowrap">&nbsp;· {split[2]}</span>}
        </div>
      </div>
      <Trailing word="Watch" />
    </button>
  );
}
