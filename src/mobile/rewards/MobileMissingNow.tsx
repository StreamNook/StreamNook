// The badges the signed-in account does not own that can be earned right now,
// above the badge wall. The strip, its order, its state and every word on it
// come ready from Rust (`get_badge_wall`, services/badge_wall.rs); this only
// lays it out at phone density.
import { Fragment, useRef, useState } from 'react';
import { Clapperboard, Coins, Dices, Eye, Info, MapPin, Sparkles, Ticket, type LucideIcon } from 'lucide-react';
import { CardChip } from '../../components/ui/CardChip';
import { RewardsPopover } from './RewardsPopover';
import type { CatchUp, EarnChip, MissingNow, MissingRow } from './rewardsModel';

const COLLAPSED_ROWS = 4;

// Drawn icons, one hue per way of earning, for kinds Twitch has no art for.
const DRAWN: Record<EarnChip['kind'], { icon: LucideIcon; tint: string }> = {
  watch: { icon: Eye, tint: 'text-sky-300' },
  subscribe: { icon: Sparkles, tint: 'text-violet-300' },
  purchase: { icon: Ticket, tint: 'text-amber-300' },
  cheer: { icon: Sparkles, tint: 'text-fuchsia-300' },
  create: { icon: Clapperboard, tint: 'text-rose-300' },
  attend: { icon: MapPin, tint: 'text-emerald-300' },
  other: { icon: Sparkles, tint: 'text-textSecondary' },
};

const ChipIcon = ({ chip }: { chip: EarnChip }) => {
  if (chip.icon_url) {
    return <img src={chip.icon_url} alt="" className="w-3.5 h-3.5 object-contain shrink-0" draggable={false} />;
  }
  const { icon, tint } = DRAWN[chip.kind];
  const Icon = chip.kind === 'purchase' && !chip.ticket ? Coins : icon;
  return <Icon size={12} className={`shrink-0 ${tint}`} />;
};

/**
 * How a badge is earned, as one glazed pill with a segment per step.
 *
 * Its own tap target: the long form of each step ("Watch 30m on 3 different
 * days") was a desktop `title`, which a finger cannot open, so a tap on the
 * pill shows it and a tap anywhere else on the row still opens the badge.
 */
const EarnPill = ({ chips, iconsOnly, title }: { chips: EarnChip[]; iconsOnly: boolean; title: string }) => {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  if (chips.length === 0) return null;
  return (
    <>
      {/* The padding, taken back by the negative margin, grows the 22 px pill
          to a finger-sized target without moving anything around it. */}
      <button
        ref={anchor}
        type="button"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`How to earn: ${chips.map((c) => c.hint).join(', ')}`}
        className="pointer-events-auto shrink-0 flex items-center -my-2.5 py-2.5 -mx-1 px-1 active:opacity-70 transition-opacity"
      >
        <span className="glaze-inset glaze-chip bg-white/[0.10] inline-flex items-center gap-1.5 h-[22px] px-2 text-[11px] font-medium leading-none text-textPrimary whitespace-nowrap">
          {chips.map((chip, i) => (
            <Fragment key={`${chip.kind}-${i}`}>
              {i > 0 && <span aria-hidden className="w-px h-3 bg-white/15" />}
              <span className="inline-flex items-center gap-1">
                <ChipIcon chip={chip} />
                {!iconsOnly && chip.text}
              </span>
            </Fragment>
          ))}
        </span>
      </button>
      <RewardsPopover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        width={264}
        role="dialog"
        label={`How to earn ${title}`}
      >
        <div className="flex flex-col gap-2 px-2.5 py-2">
          {chips.map((chip, i) => (
            <div key={`${chip.kind}-${i}`} className="flex items-start gap-2">
              <span className="mt-[3px] flex">
                <ChipIcon chip={chip} />
              </span>
              <span className="text-[12.5px] leading-snug text-textSecondary">{chip.hint}</span>
            </div>
          ))}
        </div>
      </RewardsPopover>
    </>
  );
};

/** What earning everything in the strip takes; its note says how the totals are counted. */
const CatchUpPanel = ({ catchUp }: { catchUp: CatchUp }) => {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <div className="pb-2.5">
      <div className="missing-now-catchup glaze-inset missing-now-catchup--stacked">
        <div className="flex items-center gap-1">
          <span className="missing-now-catchup-label flex-1 min-w-0">{catchUp.label}</span>
          {catchUp.notes.length > 0 && (
            // Desktop shows these on hover; a phone needs something to tap.
            <button
              ref={anchor}
              type="button"
              onClick={() => setOpen(true)}
              aria-haspopup="dialog"
              aria-expanded={open}
              aria-label="How this is counted"
              className="shrink-0 -my-2 -mr-2 w-8 h-8 flex items-center justify-center text-textMuted active:opacity-60 transition-opacity"
            >
              <Info size={13} aria-hidden />
            </button>
          )}
        </div>
        <div className="missing-now-stats">
          {catchUp.stats.map((s) => (
            <div key={s.label} className="missing-now-stat">
              <span className="missing-now-stat-value">{s.value}</span>
              <span className="missing-now-stat-label">{s.label}</span>
            </div>
          ))}
        </div>
      </div>
      <RewardsPopover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        width={280}
        align="end"
        role="dialog"
        label="How this is counted"
      >
        <div className="flex flex-col gap-2 px-2.5 py-2">
          {catchUp.notes.map((note) => (
            <p key={note} className="text-[12.5px] leading-snug text-textSecondary">
              {note}
            </p>
          ))}
        </div>
      </RewardsPopover>
    </div>
  );
};

/**
 * One earnable badge. A full-size button under the row opens the badge, and
 * the earn pill sits above it as its own button: nesting one button in another
 * is invalid and a tap on the pill would also open the sheet.
 */
const MissingRowItem = ({ row, onOpen }: { row: MissingRow; onOpen: (key: string) => void }) => (
  <div className="glass-panel relative [&:has(>.sn-missing-open:active)]:opacity-80">
    <button
      type="button"
      onClick={() => onOpen(row.key)}
      aria-label={[row.title, row.random_note, row.time_left].filter(Boolean).join(', ')}
      className="sn-missing-open absolute inset-0 w-full rounded-[inherit]"
    />
    <div className="relative pointer-events-none flex items-center gap-3 px-3 py-2">
      <img src={row.image} alt="" className="w-9 h-9 object-contain shrink-0" draggable={false} />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5 min-w-0">
          <span aria-hidden className="min-w-[3.5rem] flex-1 text-[13px] font-medium text-textPrimary truncate">
            {row.title}
          </span>
          <EarnPill chips={row.chips} iconsOnly={row.icons_only} title={row.title} />
        </div>
        {row.random_note && (
          <div className="mt-0.5" aria-hidden>
            <span className="inline-flex items-center gap-1 shrink-0 whitespace-nowrap text-[11px] text-textMuted">
              <Dices size={11} className="shrink-0" />
              {row.random_note}
            </span>
          </div>
        )}
      </div>
      {row.time_left && (
        // Closing within a day is the one time worth raising a voice.
        <span
          aria-hidden
          className={`text-[11px] font-medium shrink-0 ${row.urgent ? 'text-warning' : 'text-textSecondary'}`}
        >
          {row.time_left}
        </span>
      )}
    </div>
  </div>
);

export function MobileMissingNow({ missing, onOpen }: { missing: MissingNow; onOpen: (key: string) => void }) {
  const [expanded, setExpanded] = useState(false);
  if (missing.state === 'hidden') return null;
  const rows = expanded ? missing.rows : missing.rows.slice(0, COLLAPSED_ROWS);
  const list = missing.state === 'list';

  return (
    <section className="pb-3">
      <header className="pb-2.5">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="missing-now-title">Missing, earnable now</span>
            {list && (
              <CardChip kind="ready" flat>
                {missing.rows.length}
              </CardChip>
            )}
          </div>
          <p className="flex items-center gap-1.5 text-[11.5px] text-textMuted truncate">
            {missing.refreshing ? (
              <>
                <span aria-hidden className="missing-now-pulse" />
                <span>Checking Twitch</span>
              </>
            ) : (
              missing.closes && (
                <span>
                  The next one closes <span className="font-medium text-textSecondary">{missing.closes}</span>
                </span>
              )
            )}
          </p>
        </div>
      </header>
      {list && missing.catch_up && <CatchUpPanel catchUp={missing.catch_up} />}

      {missing.state === 'loading' ? (
        <div className="h-[52px] rounded-xl glass-panel animate-pulse" />
      ) : missing.state === 'failed' ? (
        <p className="text-[12.5px] text-textMuted">Couldn't read your badges from Twitch. Pull to try again.</p>
      ) : missing.state === 'none' ? (
        <p className="text-[12.5px] text-textMuted">You have every badge that's earnable right now.</p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {rows.map((row) => (
            <MissingRowItem key={row.key} row={row} onOpen={onOpen} />
          ))}
          {missing.rows.length > COLLAPSED_ROWS && (
            <button onClick={() => setExpanded((v) => !v)} className="self-start px-1 py-1 text-[12px] font-medium text-accent">
              {expanded ? 'Show fewer' : `Show all ${missing.rows.length}`}
            </button>
          )}
        </div>
      )}
    </section>
  );
}
