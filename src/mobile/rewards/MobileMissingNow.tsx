// The badges the signed-in account does not own that can be earned right now,
// above the badge wall. The strip, its order, its state and every word on it
// come ready from Rust (`get_badge_wall`, services/badge_wall.rs); this only
// lays it out at phone density.
import { Fragment, useState } from 'react';
import { Clapperboard, Coins, Dices, Eye, MapPin, Sparkles, Ticket, type LucideIcon } from 'lucide-react';
import { CardChip } from '../../components/ui/CardChip';
import type { EarnChip, MissingNow } from './rewardsModel';

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

/** How a badge is earned, as one glazed pill with a segment per step. */
const EarnPill = ({ chips, iconsOnly }: { chips: EarnChip[]; iconsOnly: boolean }) => {
  if (chips.length === 0) return null;
  return (
    <span className="glaze-inset glaze-chip bg-white/[0.10] inline-flex items-center gap-1.5 h-[22px] px-2 shrink-0 text-[11px] font-medium leading-none text-textPrimary whitespace-nowrap">
      {chips.map((chip, i) => (
        <Fragment key={`${chip.kind}-${i}`}>
          {i > 0 && <span aria-hidden className="w-px h-3 bg-white/15" />}
          <span className="inline-flex items-center gap-1" title={chip.hint}>
            <ChipIcon chip={chip} />
            {iconsOnly ? <span className="sr-only">{chip.text}</span> : chip.text}
          </span>
        </Fragment>
      ))}
    </span>
  );
};

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
      {list && missing.catch_up && (
        <div className="pb-2.5">
          <div
            className="missing-now-catchup glaze-inset missing-now-catchup--stacked"
            title={missing.catch_up.notes.join('\n')}
          >
            <span className="missing-now-catchup-label">{missing.catch_up.label}</span>
            <div className="missing-now-stats">
              {missing.catch_up.stats.map((s) => (
                <div key={s.label} className="missing-now-stat">
                  <span className="missing-now-stat-value">{s.value}</span>
                  <span className="missing-now-stat-label">{s.label}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {missing.state === 'loading' ? (
        <div className="h-[52px] rounded-xl glass-panel animate-pulse" />
      ) : missing.state === 'failed' ? (
        <p className="text-[12.5px] text-textMuted">Couldn't read your badges from Twitch. Pull to try again.</p>
      ) : missing.state === 'none' ? (
        <p className="text-[12.5px] text-textMuted">You have every badge that's earnable right now.</p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {rows.map((row) => (
            <button
              key={row.key}
              onClick={() => onOpen(row.key)}
              className="glass-panel flex items-center gap-3 px-3 py-2 text-left active:opacity-80"
            >
              <img src={row.image} alt="" className="w-9 h-9 object-contain shrink-0" draggable={false} />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-1.5 min-w-0">
                  <span className="min-w-[3.5rem] flex-1 text-[13px] font-medium text-textPrimary truncate">{row.title}</span>
                  <EarnPill chips={row.chips} iconsOnly={row.icons_only} />
                </div>
                {row.random_note && (
                  <div className="mt-0.5">
                    <span className="inline-flex items-center gap-1 shrink-0 whitespace-nowrap text-[11px] text-textMuted">
                      <Dices size={11} className="shrink-0" />
                      {row.random_note}
                    </span>
                  </div>
                )}
              </div>
              {row.time_left && <span className="text-[11px] font-semibold text-success shrink-0">{row.time_left}</span>}
            </button>
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
