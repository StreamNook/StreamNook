// One reward opened from a campaign sheet (a grid tile or a drop row): what it
// is, how it is earned and where, each place a tap away. Every line comes from
// Rust (services/rewards_drops.rs and services/watch_places.rs); this only
// lays it out. It replaces the campaign's content inside the same sheet, with
// a way back, rather than stacking a second sheet.
import { CaretLeft, Gift } from 'phosphor-react';
import type { TwitchCategory } from '../../types';
import type { PlaceChannel, WatchPlaces } from '../../services/watchPlaces';
import type { CampaignCard } from './rewardsModel';
import { PlaceCategoryCard, PlaceChannelCard } from './WatchPlaceCards';

export interface RewardPick {
  campaign: string;
  /** A grid tile's id, or null for a drop row. */
  tile: string | null;
  drop: string;
}

/** Channels drawn under a reward: enough to pick from without a long scroll. */
const CHANNELS_SHOWN = 8;

export function RewardDetailBody({
  card,
  pick,
  places,
  onBack,
  onBrowse,
  onWatch,
}: {
  card: CampaignCard;
  pick: RewardPick;
  places: WatchPlaces | null;
  onBack: () => void;
  onBrowse: (category: TwitchCategory) => void;
  onWatch: (channel: PlaceChannel) => void;
}) {
  const tile = pick.tile ? (card.rewards ?? []).find((t) => t.id === pick.tile) ?? null : null;
  const row = card.drops.find((d) => d.id === pick.drop) ?? null;
  const name = tile?.name ?? row?.name ?? '';
  const image = tile?.image ?? row?.image ?? '';
  const held = tile ? tile.held : !!row?.claimed;
  const locked = !held && (tile ? tile.locked : !!row?.locked);
  // Under the name: earned, else its kind for a draw's reward (the tier's
  // "5h left" belongs to the tier, shown below), else the row's own word.
  const word = held ? 'Earned' : tile?.from ? tile.kind : row?.status || tile?.kind || '';
  const categories = (places?.categories ?? []).filter((c) => c.id);
  const channels = (places?.channels ?? []).slice(0, CHANNELS_SHOWN);
  // The row's line is Twitch's progress, or what it needs while out of reach;
  // it repeats the requirement only when there is no progress yet.
  const progress = row?.detail && row.detail !== row.requirement ? row.detail : null;

  return (
    <div className="flex flex-col gap-3 pb-1">
      <button
        onClick={onBack}
        className="self-start -ml-1 h-9 flex items-center gap-1 text-[13px] font-medium text-accent active:opacity-70 min-w-0 max-w-full"
      >
        <CaretLeft size={14} weight="bold" className="shrink-0" />
        <span className="truncate">{card.name}</span>
      </button>

      <div className="flex flex-col items-center text-center">
        <div className="w-20 h-20 rounded-xl bg-surface ring-1 ring-white/10 overflow-hidden flex items-center justify-center mb-2.5">
          {image ? (
            <img
              src={image}
              alt=""
              draggable={false}
              className={`w-full h-full object-contain p-2 ${locked ? 'opacity-40 grayscale' : ''}`}
            />
          ) : (
            <Gift size={24} className="text-textMuted" />
          )}
        </div>
        <div className={`text-[16px] font-semibold leading-snug ${locked ? 'text-textSecondary' : 'text-textPrimary'}`}>
          {name}
        </div>
        {word && (
          <div className={`mt-1 text-[12px] ${held ? 'text-success font-medium' : 'text-textMuted'}`}>{word}</div>
        )}
        {tile?.from && <div className="mt-1 text-[12px] text-textMuted">{tile.from}</div>}
      </div>

      {row && (row.requirement || progress || row.bar != null) && (
        <div className="glass-panel px-3 pt-2.5 pb-3 flex flex-col gap-1.5">
          <div className="text-[11px] font-semibold uppercase tracking-wide text-textMuted">How to earn</div>
          {row.requirement && <div className="text-[14px] text-textPrimary leading-snug">{row.requirement}</div>}
          {progress && <div className="text-[12.5px] text-textSecondary leading-snug">{progress}</div>}
          {row.bar != null && !locked && (
            <div className="h-1.5 rounded-full bg-surface overflow-hidden mt-1">
              <div
                className={`h-full rounded-full ${row.done ? 'bg-success' : 'bg-accent'}`}
                style={{ width: `${row.bar}%` }}
              />
            </div>
          )}
        </div>
      )}

      {(categories.length > 0 || channels.length > 0) && (
        <div className="flex flex-col gap-1.5">
          <div className="text-[11px] font-semibold uppercase tracking-wide text-textMuted px-3">Where to watch</div>
          {categories.map((c) => (
            <PlaceCategoryCard
              key={c.id}
              place={c}
              onOpen={() => onBrowse({ id: c.id!, name: c.name, box_art_url: c.box_art_url ?? '' })}
            />
          ))}
          {channels.map((c) => (
            <PlaceChannelCard key={c.login} place={c} onWatch={() => onWatch(c)} />
          ))}
        </div>
      )}
    </div>
  );
}
