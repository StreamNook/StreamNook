// A member's accolade wall as the desktop card shows it: "{earned} / {total}
// unlocked", the earned medallions, "Show all N (M locked)" opening the locked
// ones in place (a lock on each) with "Show fewer" to close, and a progress bar.
// A tap on a medallion says what it is and how it lights, in place.
import React, { useState } from 'react';
import { Lock, Star } from 'lucide-react';
import type { AccoladeWallView } from '../../types/profileView';
import { SectionHeader, TapPanel, TapPanelHost } from './IdentityParts';
import { ACCOLADE_ICONS } from './memberIcons';
import { SECTION_TITLES, accentBorder, accoladeLabel, accoladePanel } from './profileSheetParts';
import { WALL } from './accoladeWallLayout';

/** The desktop medallion's bevel: inset light and shade, no outer glow. */
const MEDALLION_BEVEL = 'inset 1.5px 1.5px 1px rgba(255,255,255,0.35), inset -1.5px -1.5px 3px rgba(0,0,0,0.30)';

export const AccoladeWall: React.FC<{ wall: AccoladeWallView; accent?: string; className?: string }> = ({
  wall,
  accent,
  className = 'mt-5',
}) => {
  const [showAll, setShowAll] = useState(false);
  const [tapId, setTapId] = useState<string | null>(null);
  if (wall.items.length === 0) return null;
  const locked = wall.total - wall.earned;
  const items = showAll ? wall.items : wall.items.filter((a) => a.earned);
  const tapped = items.find((a) => a.id === tapId) ?? null;
  const percent = wall.total > 0 ? Math.round((wall.earned / wall.total) * 100) : 0;
  return (
    // No scroll anchoring: opening the locked ones grows the wall below the tap.
    <div
      className={`${className} glass-panel rounded-xl [overflow-anchor:none]`}
      style={{ ...accentBorder(accent, 0.3), padding: WALL.pad }}
    >
      <div style={{ height: WALL.header }}>
        <SectionHeader
          title={SECTION_TITLES.accolades}
          trailing={
            <span className="text-[11px] tabular-nums text-textMuted">{`${wall.earned} / ${wall.total} unlocked`}</span>
          }
        />
      </div>
      <TapPanelHost open={tapped !== null} onClose={() => setTapId(null)}>
        <div className="grid grid-cols-5 gap-x-1" style={{ rowGap: WALL.rowGap }}>
          {items.map((a) => {
            const Icon = ACCOLADE_ICONS[a.icon] ?? Star;
            const mystery = a.secret && !a.earned;
            const label = accoladeLabel(a);
            const plain = label.replace(/\u00ad/g, '');
            return (
              <button
                key={a.id}
                type="button"
                onClick={() => setTapId((cur) => (cur === a.id ? null : a.id))}
                aria-label={a.earned ? plain : `${plain}, locked`}
                className="flex min-w-0 flex-col items-center gap-1 transition-opacity active:opacity-70"
                style={{ height: WALL.cell }}
              >
                <span
                  className={`flex h-11 w-11 items-center justify-center rounded-full ${
                    a.earned ? '' : 'border border-white/[0.06] bg-white/[0.03] opacity-55'
                  }`}
                  style={
                    a.earned
                      ? { backgroundImage: `linear-gradient(140deg, ${a.colors[0]}, ${a.colors[1] ?? a.colors[0]})`, boxShadow: MEDALLION_BEVEL }
                      : undefined
                  }
                >
                  {a.earned ? (
                    <Icon size={18} strokeWidth={2} className="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.35)]" aria-hidden="true" />
                  ) : mystery ? (
                    <span className="text-base font-bold text-textMuted" aria-hidden="true">?</span>
                  ) : (
                    <Lock size={16} strokeWidth={2.25} className="text-textMuted" aria-hidden="true" />
                  )}
                </span>
                <span
                  className={`w-full text-center text-[10px] tracking-[-0.01em] font-medium leading-tight line-clamp-2 [hyphens:manual] ${
                    a.earned ? 'text-textSecondary' : 'text-textMuted'
                  }`}
                >
                  {label}
                </span>
              </button>
            );
          })}
        </div>
        {tapped && (
          <TapPanel panel={accoladePanel(tapped)} />
        )}
      </TapPanelHost>
      {locked > 0 && (
        <button
          type="button"
          onClick={() => setShowAll((v) => !v)}
          className="w-full text-[12px] font-medium text-textMuted"
          style={{ marginTop: WALL.sectionGap, height: WALL.toggle }}
        >
          {showAll ? 'Show fewer' : `Show all ${wall.total} (${locked} locked)`}
        </button>
      )}
      <div
        className="overflow-hidden rounded-full bg-white/[0.06]"
        style={{ marginTop: WALL.sectionGap, height: WALL.progress }}
        aria-hidden="true"
      >
        <div className="h-full rounded-full bg-accent/70" style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
};
