// The badges the signed-in account does not own that can be earned right now,
// above the badge wall. The list, its order and every fact on it come ready
// from Rust (`get_badge_standing`); this only lays it out at phone density.
import { useState } from 'react';
import { timeLeftLabel, type BadgeStanding, type MissingBadge } from '../../services/badgeStanding';
import { EarnChips, RandomDrawNote } from '../../components/badge/EarnChips';

const COLLAPSED_ROWS = 4;

export function MobileMissingNow({
  standing,
  missing,
  onOpen,
}: {
  standing: BadgeStanding | null;
  missing: MissingBadge[];
  onOpen: (key: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  if (standing?.collection === 'unavailable') return null;

  const loading =
    !standing || !standing.catalogue_ready || (standing.collection === 'partial' && standing.collection_reason !== 'fetch_failed');
  const failed = standing?.collection === 'partial' && standing.collection_reason === 'fetch_failed';
  const rows = expanded ? missing : missing.slice(0, COLLAPSED_ROWS);

  return (
    <section className="pb-3">
      <div className="flex items-baseline gap-1.5 pb-1.5">
        <span className="text-[13px] font-semibold text-textPrimary">Missing, earnable now</span>
        {!loading && !failed && missing.length > 0 && (
          <span className="text-[12px] font-semibold text-success">{missing.length}</span>
        )}
        {standing?.refreshing && <span className="text-[11.5px] text-textMuted">· checking…</span>}
      </div>

      {loading ? (
        <div className="h-[52px] rounded-xl glass-panel animate-pulse" />
      ) : failed ? (
        <p className="text-[12.5px] text-textMuted">Couldn't read your badges from Twitch. Pull to try again.</p>
      ) : missing.length === 0 ? (
        <p className="text-[12.5px] text-textMuted">You have every badge that's earnable right now.</p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {rows.map((badge) => {
            const left = timeLeftLabel(badge.ends_ms);
            return (
              <button
                key={badge.key}
                onClick={() => onOpen(badge.key)}
                className="glass-panel flex items-center gap-3 px-3 py-2 text-left active:opacity-80"
              >
                <img src={badge.image_url} alt="" className="w-9 h-9 object-contain shrink-0" draggable={false} />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5 min-w-0">
                    <span className="min-w-[3.5rem] flex-1 text-[13px] font-medium text-textPrimary truncate">{badge.title}</span>
                    <EarnChips earn={badge.earn} icons={standing?.earn_icons} />
                  </div>
                  {badge.earn.random_of && (
                    <div className="mt-0.5">
                      <RandomDrawNote of={badge.earn.random_of} />
                    </div>
                  )}
                </div>
                {left && <span className="text-[11px] font-semibold text-success shrink-0">{left}</span>}
              </button>
            );
          })}
          {missing.length > COLLAPSED_ROWS && (
            <button
              onClick={() => setExpanded((v) => !v)}
              className="self-start px-1 py-1 text-[12px] font-medium text-accent"
            >
              {expanded ? 'Show fewer' : `Show all ${missing.length}`}
            </button>
          )}
        </div>
      )}
    </section>
  );
}
