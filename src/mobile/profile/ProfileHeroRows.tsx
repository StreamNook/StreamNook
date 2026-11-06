// The rows under a member's name on their sheet: the meta line (@login, the
// badges they wear, their profile views), then the chips (their 7TV paint, their
// Atmosphere, a link to their 7TV profile). A tap on a worn badge, the paint or
// the Atmosphere names it in a panel in place; a tap outside puts it away.
import React, { useState } from 'react';
import { Eye } from 'lucide-react';
import { FallbackImage } from '../../components/FallbackImage';
import type { PaintV4 } from '../../services/paintStyle';
import type { AtmosphereView, ProfileBadge } from '../../types/profileView';
import { AtmosphereChip, PaintChip, SevenTvChip, TapPanel, TapPanelHost } from './IdentityParts';
import { panelFor, type Panel } from './profileSheetParts';

interface Tap {
  key: string;
  panel: Panel;
  art: ProfileBadge | null;
}

/** A chip's height (its line, padding and edge), so a held chip row is exactly
 *  as tall as a filled one. */
const CHIP_ROW = 23;
/** The gap above the chip row. */
const CHIP_GAP = 8;
/** The meta line with worn badges in it: a 28 px badge target pulled in by 4
 *  above and below. */
const META_ROW = 20;

export const ProfileHeroRows: React.FC<{
  login: string | null;
  /** Their worn badges, art already resolved and failed art already dropped. */
  worn: ProfileBadge[];
  views: string | null;
  paint: PaintV4 | null;
  paintColor: string;
  atmosphere: AtmosphereView | null;
  seventvUserId: string | null;
  holdStill: boolean;
  /** No answer has landed yet: the meta line and the chip row keep their space,
   *  so badges and chips arriving push nothing below them down. Released, an
   *  unneeded row collapses upward. */
  holdChips: boolean;
  onBadgeImageFailed: (badge: ProfileBadge) => void;
}> = ({ login, worn, views, paint, paintColor, atmosphere, seventvUserId, holdStill, holdChips, onBadgeImageFailed }) => {
  const [tap, setTap] = useState<Tap | null>(null);
  const toggle = (next: Tap) => setTap((cur) => (cur?.key === next.key ? null : next));
  const hasMeta = !!login || worn.length > 0 || !!views;
  const hasChips = !!paint || !!atmosphere || !!seventvUserId;
  const holding = holdChips && !hasChips;
  return (
    <TapPanelHost open={tap !== null} onClose={() => setTap(null)}>
      {(hasMeta || holdChips) && (
        // Held at a badge row's height until the answer lands, so worn badges
        // arriving under the handle push nothing down.
        <div
          className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1"
          style={holdChips ? { minHeight: META_ROW } : undefined}
        >
          {login && <span className="max-w-[60%] truncate text-[13px] leading-none text-textMuted">@{login}</span>}
          {worn.length > 0 && (
            <div className="flex items-center">
              {worn.map((b, i) => (
                <button
                  key={`${b.id}-${i}`}
                  type="button"
                  onClick={() => toggle({ key: `badge:${b.id}:${i}`, panel: panelFor('badge', b), art: b })}
                  aria-label={b.title || b.name || 'Badge'}
                  className="-my-1 flex h-7 w-7 items-center justify-center"
                >
                  <FallbackImage
                    src={b.url}
                    fallbackUrls={b.fallbacks}
                    alt=""
                    draggable={false}
                    className="h-[18px] w-[18px] object-contain"
                    onAllFailed={() => onBadgeImageFailed(b)}
                  />
                </button>
              ))}
            </div>
          )}
          {views && (
            <span className="inline-flex items-center gap-1 text-[11px] leading-none text-textMuted" aria-label={`${views} profile views`}>
              <Eye size={13} className="opacity-80" aria-hidden="true" />
              {views}
            </span>
          )}
        </div>
      )}
      {/* Always drawn, so a held row that turns out unneeded can animate shut
          (height only, which Interface > Motion collapses to instant). */}
      <div
        aria-hidden={hasChips ? undefined : true}
        style={
          hasChips
            ? undefined
            : {
                height: holding ? CHIP_GAP + CHIP_ROW : 0,
                overflow: 'hidden',
                transition: 'height 180ms cubic-bezier(0.2, 0.8, 0.2, 1)',
              }
        }
      >
        {hasChips && (
          <div className="flex flex-wrap items-center gap-1.5" style={{ marginTop: CHIP_GAP, minHeight: CHIP_ROW }}>
            {paint && (
              <PaintChip
                paint={paint}
                color={paintColor}
                still={holdStill}
                onTap={() => toggle({ key: 'paint', panel: panelFor('paint', paint), art: null })}
              />
            )}
            {atmosphere && (
              <AtmosphereChip
                atmosphere={atmosphere}
                onTap={() => toggle({ key: 'atmosphere', panel: panelFor('atmosphere', atmosphere), art: null })}
              />
            )}
            {seventvUserId && <SevenTvChip seventvUserId={seventvUserId} />}
          </div>
        )}
      </div>
      {tap && <TapPanel panel={tap.panel} art={tap.art} />}
    </TapPanelHost>
  );
};
