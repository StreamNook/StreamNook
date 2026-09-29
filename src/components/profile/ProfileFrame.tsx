// The member's equipped Frame. It is the profile card's border: the overlay
// fills the whole card, and a framed card drops its own rounded corners and
// outline so the frame's square corners are the card's (the website profile
// does the same). Reuses the Cologne nine-slice border-image styling (see
// MajorCologneChrome.css .cologne-frame). Renders nothing when none is equipped.

import { useEffect, useState } from 'react';
import { getActiveEquipment, getCosmeticBySlug } from '../../services/supabaseService';
import { resolveCosmeticAsset } from '../cosmeticAssets';
import type { ActiveEquipment } from '../../services/cosmetics/types';

/** The member's equipped frame art, or null when none is equipped. */
export function useProfileFrameUrl(userId: string | null | undefined): string | null {
  const [equipment, setEquipment] = useState<ActiveEquipment>({});

  useEffect(() => {
    setEquipment({});
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

  const frameSlug = equipment.frame;
  const cosmetic = frameSlug ? getCosmeticBySlug(frameSlug) : null;
  return cosmetic ? resolveCosmeticAsset(cosmetic) : null;
}

/** The frame over its positioned parent, the whole profile card. */
export function ProfileFrame({ url }: { url: string | null }) {
  if (!url) return null;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 z-[30]"
      style={{
        borderStyle: 'solid',
        borderWidth: '18px 14px',
        borderImageSource: `url(${url})`,
        borderImageSlice: '199 159 199 159',
        borderImageWidth: '18px 14px',
        borderImageRepeat: 'stretch',
      }}
    />
  );
}
