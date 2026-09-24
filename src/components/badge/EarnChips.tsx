import { Fragment } from 'react';
import { Clapperboard, Coins, Dices, Eye, MapPin, Sparkles, Ticket, type LucideIcon } from 'lucide-react';
import { earnChipHint, earnChipText, type EarnPath, type EarnStep } from '../../services/badgeStanding';

// Drawn icons, one hue per way of earning, for the kinds Twitch has no art for.
// Subscribing and Bits use Twitch's own badge art when the standing carries it.
const DRAWN: Record<EarnStep['kind'], { icon: LucideIcon; tint: string }> = {
  watch: { icon: Eye, tint: 'text-sky-300' },
  subscribe: { icon: Sparkles, tint: 'text-violet-300' },
  purchase: { icon: Ticket, tint: 'text-amber-300' },
  cheer: { icon: Sparkles, tint: 'text-fuchsia-300' },
  create: { icon: Clapperboard, tint: 'text-rose-300' },
  attend: { icon: MapPin, tint: 'text-emerald-300' },
  other: { icon: Sparkles, tint: 'text-textSecondary' },
};

// Past this many steps the pill shows icons only (each keeps its tooltip), so
// its width stays bounded however a badge is earned.
const ICONS_ONLY_FROM = 3;

const StepIcon = ({ step, art }: { step: EarnStep; art?: string }) => {
  if (art) return <img src={art} alt="" className="w-3.5 h-3.5 object-contain shrink-0" draggable={false} />;
  const { icon, tint } = DRAWN[step.kind];
  // A paid step that is not a ticket reads better as coins.
  const Icon = step.kind === 'purchase' && !step.ticket ? Coins : icon;
  return <Icon size={12} className={`shrink-0 ${tint}`} />;
};

/** How a badge is earned, as ONE glazed pill: each step (what you pay or sub,
 *  then what you watch) is a segment, divided by a hairline. One pill rather
 *  than a chip per step keeps several steps compact enough to sit beside a
 *  title and reads as one requirement. The classification is Rust's
 *  (badge_earn.rs), and so is the Twitch art (`icons`, by step kind). A random
 *  draw is shown separately with `RandomDrawNote`. */
export const EarnChips = ({ earn, icons }: { earn: EarnPath; icons?: Record<string, string> }) => {
  if (earn.steps.length === 0) return null;
  const iconsOnly = earn.steps.length >= ICONS_ONLY_FROM;
  return (
    <span className="glaze-inset glaze-chip bg-white/[0.10] inline-flex items-center gap-1.5 h-[22px] px-2 shrink-0 text-[11px] font-medium leading-none text-textPrimary whitespace-nowrap">
      {earn.steps.map((step, i) => (
        <Fragment key={`${step.kind}-${i}`}>
          {i > 0 && <span aria-hidden className="w-px h-3 bg-white/15" />}
          <span className="inline-flex items-center gap-1" title={earnChipHint(step)}>
            <StepIcon step={step} art={icons?.[step.kind]} />
            {iconsOnly ? <span className="sr-only">{earnChipText(step)}</span> : earnChipText(step)}
          </span>
        </Fragment>
      ))}
    </span>
  );
};

/** "1 of 3 at random" for a badge drawn from a pool, as quiet inline text. */
export const RandomDrawNote = ({ of }: { of: number }) => (
  <span
    className="inline-flex items-center gap-1 shrink-0 whitespace-nowrap text-[11px] text-textMuted"
    title={`You get one of ${of} badges, drawn at random`}
  >
    <Dices size={11} className="shrink-0" />1 of {of} at random
  </span>
);
