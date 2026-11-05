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
  onBadgeImageFailed: (badge: ProfileBadge) => void;
}> = ({ login, worn, views, paint, paintColor, atmosphere, seventvUserId, holdStill, onBadgeImageFailed }) => {
  const [tap, setTap] = useState<Tap | null>(null);
  const toggle = (next: Tap) => setTap((cur) => (cur?.key === next.key ? null : next));
  const hasMeta = !!login || worn.length > 0 || !!views;
  const hasChips = !!paint || !!atmosphere || !!seventvUserId;
  if (!hasMeta && !hasChips) return null;
  return (
    <TapPanelHost open={tap !== null} onClose={() => setTap(null)}>
      {hasMeta && (
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
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
      {hasChips && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
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
      {tap && <TapPanel panel={tap.panel} art={tap.art} />}
    </TapPanelHost>
  );
};
