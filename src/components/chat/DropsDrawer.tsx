// The drawer over chat: every campaign you can earn on this channel, each
// reward with its own bar, Claim on the ones that are ready and Connect where
// the game account still needs linking. A sibling of the bar (never inside its
// glass), so its own blur reads the chat behind it.
import { motion } from 'framer-motion';
import { Check } from 'phosphor-react';
import type { HereCampaign, HereDrop, WatchDrops } from '../../stores/watchDropsStore';
import { openExternal } from '../../utils/openExternal';
import { DropArt } from './DropArt';

const EASE = [0.2, 0.8, 0.2, 1] as const;

interface Props {
  view: WatchDrops;
  maxHeight: number;
  claimingId: string | null;
  onClaim: (dropId: string, instanceId: string | null) => void;
}

export default function DropsDrawer({ view, maxHeight, claimingId, onClaim }: Props) {
  const left = view.campaigns.reduce((n, c) => n + c.drops.filter((d) => !d.claimed).length, 0);
  return (
    <motion.div
      role="dialog"
      aria-label="Drops on this channel"
      initial={{ opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.985 }}
      transition={{ duration: 0.18, ease: EASE }}
      style={{ maxHeight }}
      className="liquid-glass-panel drops-drawer absolute bottom-full left-2 right-2 flex origin-bottom flex-col overflow-hidden"
    >
      <div className="flex items-baseline justify-between gap-3 px-3 pb-1.5 pt-2.5">
        <span className="text-[11px] font-medium uppercase tracking-wider text-textSecondary">Drops on this channel</span>
        <span className="shrink-0 text-[11px] tabular-nums text-textSecondary">
          {left === 1 ? '1 reward left' : `${left} rewards left`}
        </span>
      </div>
      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-1.5 pb-1.5">
        {view.campaigns.map((campaign, i) => (
          <CampaignBlock
            key={campaign.id}
            campaign={campaign}
            first={i === 0}
            claimingId={claimingId}
            onClaim={onClaim}
          />
        ))}
      </div>
    </motion.div>
  );
}

function CampaignBlock({
  campaign,
  first,
  claimingId,
  onClaim,
}: {
  campaign: HereCampaign;
  first: boolean;
  claimingId: string | null;
  onClaim: Props['onClaim'];
}) {
  const meta = [campaign.claimed_label, campaign.ends].filter(Boolean).join(' · ');
  return (
    <section className={first ? '' : 'mt-1 border-t border-white/[0.06] pt-1'}>
      <div className="flex items-center gap-2 px-1.5 pb-1 pt-1.5">
        <DropArt image={campaign.art} size={18} />
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium leading-4 text-textPrimary">{campaign.name}</span>
        {meta && <span className="shrink-0 text-[11px] leading-4 tabular-nums text-textSecondary">{meta}</span>}
      </div>
      {campaign.drops.map((drop) => (
        <DropRow
          key={drop.id}
          drop={drop}
          linkUrl={campaign.link_url}
          claiming={claimingId === drop.id}
          onClaim={onClaim}
        />
      ))}
    </section>
  );
}

function DropRow({
  drop,
  linkUrl,
  claiming,
  onClaim,
}: {
  drop: HereDrop;
  linkUrl: string | null;
  claiming: boolean;
  onClaim: Props['onClaim'];
}) {
  const fill = drop.ready ? 'bg-success' : 'bg-accent-on-surface';
  const percent = Math.max(0, Math.min(100, drop.bar ?? 0));
  return (
    <div className="flex min-h-[40px] items-center gap-2.5 rounded-lg px-1.5 py-1.5 transition-colors duration-150 hover:bg-white/[0.04]">
      <DropArt image={drop.image} size={26} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className={`min-w-0 truncate text-[12.5px] leading-4 ${drop.claimed ? 'text-textSecondary' : 'text-textPrimary'}`}>
            {drop.name}
          </span>
          {drop.lead && (
            <span className="shrink-0 text-[10px] font-medium uppercase tracking-wider text-accent-on-surface">Next</span>
          )}
        </div>
        {drop.bar !== null && !drop.claimed && (
          <div className="mt-1.5 h-[3px] overflow-hidden rounded-full bg-textPrimary/[0.10]">
            <div className={`drops-bar-fill h-full rounded-full ${fill}`} style={{ width: `${percent}%` }} />
          </div>
        )}
        {drop.detail && !drop.claimed && (
          <div className="mt-1 truncate text-[11px] leading-[14px] text-textSecondary">{drop.detail}</div>
        )}
      </div>
      <div className="flex min-w-[64px] shrink-0 justify-end">
        <Trailing drop={drop} linkUrl={linkUrl} claiming={claiming} onClaim={onClaim} />
      </div>
    </div>
  );
}

const PILL = 'min-w-[64px] rounded-full border px-2.5 text-center text-[11.5px] font-semibold leading-[20px] transition-opacity duration-150 disabled:opacity-60';

function Trailing({
  drop,
  linkUrl,
  claiming,
  onClaim,
}: {
  drop: HereDrop;
  linkUrl: string | null;
  claiming: boolean;
  onClaim: Props['onClaim'];
}) {
  // A claim already sent waits for Twitch, and a drop Twitch would not grant
  // needs the game account linked: neither offers Claim again.
  if (drop.pending && !drop.claimed) {
    return (
      <button type="button" disabled className={`${PILL} border-success/40 text-success`}>
        Claiming
      </button>
    );
  }
  if (drop.connect && !drop.claimed) {
    return linkUrl ? (
      <button
        type="button"
        onClick={() => void openExternal(linkUrl)}
        className={`${PILL} border-white/15 text-textPrimary hover:border-white/25`}
      >
        Connect
      </button>
    ) : (
      <span className="text-right text-[11.5px] leading-4 text-textSecondary">{drop.status}</span>
    );
  }
  if (drop.ready && !drop.claimed) {
    return (
      <button
        type="button"
        disabled={claiming}
        onClick={() => onClaim(drop.id, drop.instance_id)}
        className={`${PILL} border-success/60 text-success`}
      >
        {claiming ? 'Claiming' : drop.claim_label}
      </button>
    );
  }
  if (drop.claimed) {
    return (
      <span className="flex items-center gap-1 text-[11.5px] leading-4 text-success">
        <Check size={12} weight="bold" aria-hidden />
        Claimed
      </span>
    );
  }
  return <span className="text-right text-[11.5px] leading-4 tabular-nums text-textSecondary">{drop.status}</span>;
}
