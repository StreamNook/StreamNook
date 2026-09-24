// A member's earned Relics on their profile: the featured Relic (from the
// equipment relic_primary slot) shown large with its name + source, then a capped
// row of other owned relics as icons. "View all" opens the Reliquary (the full
// collection). Reads owned relics from the cosmetics registry and the featured one
// from the equipment model. Renders nothing for members with no relics.

import { useContext, useEffect, useState } from 'react';
import { Gem } from 'lucide-react';
import { ProfileAccentContext } from '../settings/profileAccentContext';
import {
  getActiveEquipment,
  getAllCosmetics,
  getOwnedCosmeticSlugs,
} from '../../services/supabaseService';
import { resolveCosmeticAsset } from '../cosmeticAssets';
import { Tooltip } from '../ui/Tooltip';
import { Reliquary } from './Reliquary';
import type { ActiveEquipment } from '../../services/cosmetics/types';

// Non-featured relics shown inline before collapsing into "+N" / the Reliquary.
const MAX_ICONS = 6;

export function RelicStrip({ userId }: { userId: string | null | undefined }) {
  const [equipment, setEquipment] = useState<ActiveEquipment>({});
  const [showAll, setShowAll] = useState(false);

  useEffect(() => {
    if (!userId) return;
    let alive = true;
    getActiveEquipment(userId)
      .then((e) => {
        if (alive) setEquipment(e);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [userId]);

  if (!userId) return null;
  return <RelicStripBody userId={userId} equipment={equipment} showAll={showAll} setShowAll={setShowAll} />;
}

function RelicStripBody({
  userId,
  equipment,
  showAll,
  setShowAll,
}: {
  userId: string;
  equipment: ActiveEquipment;
  showAll: boolean;
  setShowAll: (v: boolean) => void;
}) {
  const accentRgb = useContext(ProfileAccentContext);
  const owned = getOwnedCosmeticSlugs(userId);
  const relics = getAllCosmetics().filter((c) => c.kind === 'relic' && owned.has(c.slug));
  if (relics.length === 0) return null;

  // The equipped relic leads; fall back to the first owned when none is equipped.
  const featuredSlug = equipment.relic_primary ?? relics[0].slug;
  const featured = relics.find((r) => r.slug === featuredSlug) ?? relics[0];
  const others = relics.filter((r) => r.slug !== featured.slug);
  const featuredAsset = resolveCosmeticAsset(featured);

  // Only the featured relic gets the name + description; the rest are compact
  // icons so the strip stays clean no matter how many are earned. Overflow beyond
  // the cap collapses into the Reliquary.
  const shownOthers = others.slice(0, MAX_ICONS);
  const overflow = others.length - shownOthers.length;

  return (
    <div className="settings-card mb-3 p-3.5" style={accentRgb ? { borderColor: `rgba(${accentRgb}, 0.3)` } : undefined}>
      <div className="mb-2.5 flex items-center justify-between">
        <h4 className="flex items-center gap-1.5 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">
          <Gem size={14} className="text-textMuted" />
          Relics
        </h4>
        {relics.length > 1 && (
          <button
            onClick={() => setShowAll(true)}
            className="text-xs text-textSecondary transition-colors hover:text-textPrimary"
          >
            View all ({relics.length})
          </button>
        )}
      </div>

      <div className="glass-tile flex items-center gap-3 p-3">
        {featuredAsset && (
          <img
            src={featuredAsset}
            alt={featured.name}
            draggable={false}
            className="h-24 w-24 shrink-0 rounded-lg object-contain"
          />
        )}
        <div className="min-w-0">
          <div className="truncate text-sm font-medium text-textPrimary">{featured.name}</div>
          {featured.description && (
            <div className="truncate text-xs text-textSecondary">{featured.description}</div>
          )}
        </div>
      </div>

      {others.length > 0 && (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {shownOthers.map((relic) => {
            const asset = resolveCosmeticAsset(relic);
            if (!asset) return null;
            return (
              <Tooltip key={relic.slug} content={relic.name} side="top">
                <img
                  src={asset}
                  alt={relic.name}
                  draggable={false}
                  className="h-12 w-12 rounded-lg object-contain opacity-80"
                />
              </Tooltip>
            );
          })}
          {overflow > 0 && (
            <button
              onClick={() => setShowAll(true)}
              className="h-12 rounded-lg px-2 text-xs text-textSecondary transition-colors hover:text-textPrimary"
            >
              +{overflow}
            </button>
          )}
        </div>
      )}

      {showAll && <Reliquary relics={relics} onClose={() => setShowAll(false)} />}
    </div>
  );
}
