// The drawer over chat: every campaign you can earn on this channel. Rewards
// sit as glazed tiles in rows of three, each with its own picture, bar and
// time; the next reward is ringed, a ready one turns green with Claim. Every
// row is full: two left over become two wider tiles, one left over a
// horizontal bar, and a campaign with one reward draws it as one wide tile.
// A sibling of the bar (never inside its glass), so its own blur reads the
// chat behind it.
import { motion } from 'framer-motion';
import { Check } from 'phosphor-react';
import type { HereCampaign, HereDrop, WatchDrops } from '../../stores/watchDropsStore';
import { ACCENT_BUTTON, SUCCESS_FILL } from '../ui/glazeButtons';
import { DropArt } from './DropArt';
import { splitStatus } from './dropsStatusText';

const EASE = [0.2, 0.8, 0.2, 1] as const;

interface Props {
  view: WatchDrops;
  /** Motion Off or Performance mode: appear and go at once. */
  instant?: boolean;
  maxHeight: number;
  claimingId: string | null;
  onClaim: (dropId: string, instanceId: string | null) => void;
}

type OnClaim = Props['onClaim'];

export default function DropsDrawer({ view, instant, maxHeight, claimingId, onClaim }: Props) {
  return (
    <motion.div
      role="dialog"
      aria-label="Drops on this channel"
      initial={{ opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1, transition: instant ? { duration: 0 } : { duration: 0.17, ease: EASE } }}
      exit={{ opacity: 0, y: 6, scale: 0.985, transition: instant ? { duration: 0 } : { duration: 0.11, ease: EASE } }}
      style={{ maxHeight }}
      className="liquid-glass-panel drops-drawer absolute bottom-full left-2 right-2 flex origin-bottom flex-col overflow-hidden"
    >
      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto p-3">
        <DrawerBody view={view} claimingId={claimingId} onClaim={onClaim} />
      </div>
    </motion.div>
  );
}

/** What the drawer lists, without its own glass or scroll: desktop's drawer
 *  wraps it in a panel over chat, the phones' lip in a bottom sheet. */
export function DrawerBody({ view, claimingId, onClaim }: Pick<Props, 'view' | 'claimingId' | 'onClaim'>) {
  const left = view.campaigns.reduce((n, c) => n + c.drops.filter((d) => !d.claimed).length, 0);
  const one = view.campaigns.length === 1 ? view.campaigns[0] : null;
  return (
    <>
        {one ? (
          <>
            <CampaignHeader campaign={one} />
            <Rewards campaign={one} claimingId={claimingId} onClaim={onClaim} solo />
          </>
        ) : (
          <>
            <div className="flex items-baseline justify-between gap-3">
              <span className="text-[10.5px] font-medium uppercase tracking-[0.08em] text-textSecondary">
                Drops on this channel
              </span>
              <span className="shrink-0 text-[11px] tabular-nums text-textSecondary">
                {left === 1 ? '1 left' : `${left} left`}
              </span>
            </div>
            {view.campaigns.map((campaign, i) =>
              campaign.drops.length === 1 ? (
                <WideTile
                  key={campaign.id}
                  campaign={campaign}
                  drop={campaign.drops[0]}
                  claiming={claimingId === campaign.drops[0].id}
                  onClaim={onClaim}
                />
              ) : (
                // A campaign with several rewards gets its own header; a rule
                // above it parts it from whatever came before.
                <section
                  key={campaign.id}
                  className={i === 0 ? 'mt-3' : 'mt-3 border-t border-white/[0.06] pt-3'}
                >
                  <CampaignHeader campaign={campaign} />
                  <Rewards campaign={campaign} claimingId={claimingId} onClaim={onClaim} />
                </section>
              ),
            )}
          </>
        )}
    </>
  );
}

/** The game's box art when the campaign has a category, else its own art. */
function CampaignArt({ campaign, small }: { campaign: HereCampaign; small?: boolean }) {
  const src = campaign.box_art || campaign.art;
  const box = small ? { width: 9, height: 12, borderRadius: 2 } : { width: 24, height: 32, borderRadius: 4 };
  if (!campaign.box_art) return <DropArt image={src} size={small ? 12 : 24} radius={small ? 3 : 5} />;
  return <img src={src} alt="" draggable={false} style={box} className="drops-art shrink-0 object-cover" />;
}

/** Name, "Game · 8d left", and "1 of 3" with one segment per reward. */
function CampaignHeader({ campaign }: { campaign: HereCampaign }) {
  const sub = [campaign.game_name !== campaign.name ? campaign.game_name : '', campaign.ends].filter(Boolean).join(' · ');
  const claimed = campaign.drops.filter((d) => d.claimed).length;
  const total = campaign.drops.length;
  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <CampaignArt campaign={campaign} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-medium leading-4 text-textPrimary">{campaign.name}</div>
        {sub && <div className="mt-0.5 truncate text-[11px] leading-[14px] text-textSecondary">{sub}</div>}
      </div>
      {campaign.started && total > 1 && (
        <div className="flex shrink-0 items-center gap-[7px] self-start pt-0.5">
          <span className="text-[11px] leading-4 tabular-nums text-textSecondary">
            {claimed} of {total}
          </span>
          {total <= 8 && (
            <span aria-hidden className="flex gap-[3px]">
              {campaign.drops.map((d) => (
                <i
                  key={d.id}
                  className={`block h-1 w-3 rounded-full transition-colors duration-200 ${d.claimed ? 'bg-success' : 'bg-white/[0.16]'}`}
                />
              ))}
            </span>
          )}
        </div>
      )}
    </div>
  );
}

function Rewards({
  campaign,
  claimingId,
  onClaim,
  solo,
}: {
  campaign: HereCampaign;
  claimingId: string | null;
  onClaim: OnClaim;
  solo?: boolean;
}) {
  if (solo && campaign.drops.length === 1) {
    const drop = campaign.drops[0];
    return (
      <div className="mt-3">
        <WideTile campaign={campaign} drop={drop} claiming={claimingId === drop.id} onClaim={onClaim} big />
      </div>
    );
  }
  return (
    <div className="mt-3 flex flex-col gap-2">
      {rewardRows(campaign.drops).map((row) =>
        row.length === 1 ? (
          <WideTile
            key={row[0].id}
            campaign={campaign}
            drop={row[0]}
            claiming={claimingId === row[0].id}
            onClaim={onClaim}
            bare
          />
        ) : (
          <div key={row[0].id} className={`grid gap-2 ${row.length === 3 ? 'grid-cols-3' : 'grid-cols-2'}`}>
            {row.map((drop) => (
              <Tile
                key={drop.id}
                drop={drop}
                claiming={claimingId === drop.id}
                onClaim={onClaim}
                pair={row.length === 2}
              />
            ))}
          </div>
        ),
      )}
    </div>
  );
}

/** Rows of three tiles; what is left over after them becomes a row of two
 *  wider tiles, or one horizontal bar, so every row is full. */
export function rewardRows<T>(drops: T[]): T[][] {
  const rows: T[][] = [];
  for (let i = 0; i < drops.length; i += 3) rows.push(drops.slice(i, i + 3));
  return rows;
}

function tileState(drop: HereDrop): string {
  if (drop.claimed) return 'is-claimed';
  if (drop.ready && !drop.pending) return 'is-ready';
  if (drop.lead) return 'is-next';
  return '';
}

/** One reward in a row of three (square art) or of two (wider art, the
 *  picture fitted inside it): the name, bar, then its time, or what it waits
 *  on. */
function Tile({
  drop,
  claiming,
  onClaim,
  pair,
}: {
  drop: HereDrop;
  claiming: boolean;
  onClaim: OnClaim;
  pair?: boolean;
}) {
  return (
    <div className={`glaze-inset drops-tile flex min-w-0 flex-col gap-1.5 p-2 ${tileState(drop)}`}>
      <DropArt image={drop.image} size="fill" aspect={pair ? '16 / 10' : undefined} />
      {/* Three across, a name gets two lines (the same two in every tile, so
          the bars line up); two across there is room for it on one. */}
      <div
        className={`text-[11px] leading-[14px] text-textPrimary/90 ${pair ? 'truncate text-[11.5px]' : 'line-clamp-2 h-7'}`}
      >
        {drop.name}
      </div>
      <Bar drop={drop} />
      <div className="flex min-h-[22px] items-center">
        <Trailing drop={drop} claiming={claiming} onClaim={onClaim} wide />
      </div>
    </div>
  );
}

/** A reward as one horizontal tile: art beside the name and time, a
 *  full-width bar. `big` is a campaign's only reward (bigger art, and the
 *  minutes or the claim under the bar); between campaigns it carries its
 *  campaign's line instead of a header; `bare` is the one left over under a
 *  campaign's rows of tiles, whose header is already above it. */
function WideTile({
  campaign,
  drop,
  claiming,
  onClaim,
  big,
  bare,
}: {
  campaign: HereCampaign;
  drop: HereDrop;
  claiming: boolean;
  onClaim: OnClaim;
  big?: boolean;
  bare?: boolean;
}) {
  const { time } = splitStatus(drop.status);
  const waiting = !drop.claimed && !drop.ready && !drop.pending && !drop.watched;
  const campaignLine = bare ? '' : [campaign.name, campaign.ends].filter(Boolean).join(' · ');
  const art = big ? 64 : bare ? 40 : 52;
  return (
    <div className={`glaze-inset drops-tile flex items-center gap-[11px] p-2.5 ${tileState(drop)} ${big || bare ? '' : 'mt-2.5'}`}>
      <DropArt image={drop.image} size={art} radius={big ? 10 : 8} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-baseline justify-between gap-2">
          <span className={`min-w-0 truncate font-medium text-textPrimary ${big ? 'text-[13px]' : 'text-[12.5px]'}`}>
            {drop.name}
          </span>
          {waiting && time && (
            <span className="shrink-0 whitespace-nowrap text-[11px] tabular-nums text-textSecondary">
              <span className={`font-semibold text-textPrimary ${big ? 'text-[15px]' : 'text-[13.5px]'}`}>{time}</span> left
            </span>
          )}
          {!big && !waiting && <Trailing drop={drop} claiming={claiming} onClaim={onClaim} />}
        </div>
        {!big && campaignLine && (
          <div className="mt-0.5 flex min-w-0 items-center gap-[5px]">
            <CampaignArt campaign={campaign} small />
            <span className="truncate text-[11px] leading-[14px] text-textSecondary">{campaignLine}</span>
          </div>
        )}
        <div className={big ? 'mb-1.5 mt-2.5' : 'mt-2'}>
          <Bar drop={drop} thick={big} />
        </div>
        {big && (
          <div className="flex min-h-[24px] items-center justify-between gap-2">
            <span className={`truncate text-[11px] tabular-nums ${drop.ready && !drop.claimed ? 'text-success' : 'text-textSecondary'}`}>
              {drop.claimed
                ? 'Claimed'
                : drop.ready && !drop.pending
                  ? 'Ready to claim'
                  : drop.watched
                    ? 'Watched, waiting on Twitch'
                    : drop.bar !== null
                      ? `${drop.current_minutes} of ${drop.required_minutes} min watched`
                      : drop.status}
            </span>
            {!waiting && !drop.watched && <Trailing drop={drop} claiming={claiming} onClaim={onClaim} />}
          </div>
        )}
        {drop.detail && !drop.claimed && (
          <div className="mt-1 truncate text-[11px] leading-[14px] text-textSecondary">{drop.detail}</div>
        )}
      </div>
    </div>
  );
}

function Bar({ drop, thick }: { drop: HereDrop; thick?: boolean }) {
  if (drop.bar === null || drop.claimed) return <div className={thick ? 'h-1' : 'h-[3px]'} />;
  const percent = Math.max(0, Math.min(100, drop.bar));
  const ready = drop.ready && !drop.pending;
  return (
    <div className={`${thick ? 'h-1' : 'h-[3px]'} overflow-hidden rounded-full bg-textPrimary/[0.09]`}>
      <div className={`drops-bar-fill h-full rounded-full ${ready ? 'is-ready' : ''}`} style={{ width: `${percent}%` }} />
    </div>
  );
}

const CAPSULE = '!text-[11.5px] px-3 leading-[22px] text-center';

/** What a reward offers or waits on: Claim, Claiming, Watched, Claimed, or
 *  its time left. `wide` stretches a button across its tile. */
function Trailing({
  drop,
  claiming,
  onClaim,
  wide,
}: {
  drop: HereDrop;
  claiming: boolean;
  onClaim: OnClaim;
  wide?: boolean;
}) {
  const span = wide ? 'w-full' : 'min-w-[64px] shrink-0';
  // A claim already sent waits for Twitch, and one Twitch took but has not
  // granted is retried by the auto-claim: neither offers Claim again.
  if (drop.claimed) {
    return (
      <span className="flex items-center gap-1 text-[11px] leading-4 text-success">
        <Check size={11} weight="bold" aria-hidden />
        Claimed
      </span>
    );
  }
  if (drop.pending) {
    return (
      <button type="button" disabled style={SUCCESS_FILL} className={`${ACCENT_BUTTON} ${CAPSULE} ${span}`}>
        Claiming
      </button>
    );
  }
  if (drop.watched) {
    return <span className="truncate text-[11px] leading-4 text-textSecondary">{drop.status}</span>;
  }
  if (drop.ready) {
    return (
      <button
        type="button"
        disabled={claiming}
        onClick={() => onClaim(drop.id, drop.instance_id)}
        style={SUCCESS_FILL}
        className={`${ACCENT_BUTTON} ${CAPSULE} ${span}`}
      >
        {claiming ? 'Claiming' : drop.claim_label}
      </button>
    );
  }
  const { time } = splitStatus(drop.status);
  if (!time) return <span className="truncate text-[11px] leading-[14px] text-textSecondary">{drop.status}</span>;
  return (
    <span className="whitespace-nowrap text-[11px] tabular-nums text-textSecondary">
      <span className={drop.lead ? 'font-semibold text-textPrimary' : ''}>{time}</span> left
    </span>
  );
}
