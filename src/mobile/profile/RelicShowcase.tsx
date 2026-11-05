// A member's relics as the desktop card's strip shows them: the first one big
// with its name and description, the next six small, "+N" for the rest, and
// "View all (N)" opening the Reliquary. The Reliquary is a MobileSheet, so the
// system Back button closes it before the profile sheet under it.
import React, { useState } from 'react';
import { FallbackImage } from '../../components/FallbackImage';
import type { ProfileBadge } from '../../types/profileView';
import { SectionHeader } from './IdentityParts';
import { MobileSheet } from '../ui/MobileSheet';
import { SECTION_TITLES, accentBorder } from './profileSheetParts';

/** Relics drawn small before the rest collapse into "+N". */
const MAX_SMALL = 6;

const relicName = (r: ProfileBadge): string => r.title || r.name || 'Relic';

export const RelicShowcase: React.FC<{
  /** Art already resolved and failed art already dropped. */
  relics: ProfileBadge[];
  accent?: string;
  onImageFailed: (relic: ProfileBadge) => void;
}> = ({ relics, accent, onImageFailed }) => {
  const [viewAll, setViewAll] = useState(false);
  if (relics.length === 0) return null;
  const [featured, ...others] = relics;
  const small = others.slice(0, MAX_SMALL);
  const overflow = others.length - small.length;
  return (
    <>
      <div className="mt-5 glass-panel rounded-xl p-3.5" style={accentBorder(accent, 0.3)}>
        <SectionHeader
          title={SECTION_TITLES.relics}
          className="mb-2.5"
          trailing={
            relics.length > 1 ? (
              <button
                type="button"
                onClick={() => setViewAll(true)}
                className="relative -my-3 py-3 pl-4 text-[12px] text-textSecondary"
              >
                {`View all (${relics.length})`}
              </button>
            ) : undefined
          }
        />
        <div className="glass-tile flex items-center gap-3 p-3">
          <FallbackImage
            src={featured.url}
            fallbackUrls={featured.fallbacks}
            alt={relicName(featured)}
            draggable={false}
            className="h-24 w-24 shrink-0 rounded-lg object-contain"
            onAllFailed={() => onImageFailed(featured)}
          />
          <div className="min-w-0">
            <div className="text-sm font-medium text-textPrimary">{relicName(featured)}</div>
            {featured.description && (
              <div className="mt-0.5 line-clamp-3 text-xs text-textSecondary">{featured.description}</div>
            )}
          </div>
        </div>
        {others.length > 0 && (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {small.map((r, i) => (
              <FallbackImage
                key={`${r.id}-${i}`}
                src={r.url}
                fallbackUrls={r.fallbacks}
                alt={relicName(r)}
                draggable={false}
                className="h-12 w-12 rounded-lg object-contain opacity-80"
                onAllFailed={() => onImageFailed(r)}
              />
            ))}
            {overflow > 0 && (
              <button
                type="button"
                onClick={() => setViewAll(true)}
                className="h-12 rounded-lg px-2 text-xs text-textSecondary"
                aria-label={`${overflow} more relics`}
              >
                {`+${overflow}`}
              </button>
            )}
          </div>
        )}
      </div>
      <MobileSheet open={viewAll} onClose={() => setViewAll(false)} title="Reliquary" maxHeightFraction={0.8}>
        <div className="flex flex-col gap-3 pb-2">
          {relics.map((r, i) => (
            <div key={`${r.id}-${i}`} className="flex items-center gap-3">
              <FallbackImage
                src={r.url}
                fallbackUrls={r.fallbacks}
                alt={relicName(r)}
                draggable={false}
                className="h-16 w-16 shrink-0 rounded-lg object-contain"
              />
              <div className="min-w-0">
                <div className="text-sm font-medium text-textPrimary">{relicName(r)}</div>
                {r.description && <div className="text-xs text-textSecondary">{r.description}</div>}
              </div>
            </div>
          ))}
        </div>
      </MobileSheet>
    </>
  );
};
