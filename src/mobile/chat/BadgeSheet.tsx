// The sheet a badge opens when tapped on the phone (any chat row, the +N fold,
// a profile): the badge large, its name, where it is from, and what Rust knows
// about it. It reads the same store the desktop spotlight card does
// (`openBadgeInfo`), mounted once in the phone shell. A tap outside, a drag
// down on the grabber, or Back closes it.
import { useState } from 'react';
import { Check, ExternalLink } from 'lucide-react';

import { MobileSheet } from '../ui/MobileSheet';
import { BadgeArt } from '../../components/chat/BadgeHoverCard';
import { useBadgeSpotlightStore } from '../../stores/badgeSpotlightStore';
import { badgeCardFacts, useBadgeDetail, type BadgeCardFacts } from '../../hooks/useBadgeCard';
import type { BadgeRef } from '../../utils/badgeSpotlight';
import { openExternal } from '../../utils/openExternal';

/** Art box: the badge large enough to read, as the emote sheet's. */
const ART_PX = 112;

export function BadgeSheet() {
  const badge = useBadgeSpotlightStore((s) => s.badge);
  const close = useBadgeSpotlightStore((s) => s.close);
  // The last badge stays drawn while the sheet's exit plays.
  const [shown, setShown] = useState<BadgeRef | null>(null);
  if (badge && badge !== shown) setShown(badge);
  if (!shown) return null;
  return (
    <Lookup
      key={`${shown.provider}:${shown.id}:${shown.title}`}
      badge={shown}
      open={badge !== null}
      onClose={close}
      onExited={() => {
        if (useBadgeSpotlightStore.getState().badge === null) setShown(null);
      }}
    />
  );
}

/** One badge's lookup: keyed by the badge, so another tap starts a fresh one. */
function Lookup({ badge, open, onClose, onExited }: { badge: BadgeRef; open: boolean; onClose: () => void; onExited: () => void }) {
  const { detail, settled } = useBadgeDetail(badge);
  // Opens once Rust has answered (or its short wait ran out), so the sheet
  // never grows under the finger.
  return (
    <MobileSheet open={open && settled} onClose={onClose} onExited={onExited}>
      <BadgeSheetBody facts={badgeCardFacts(badge, detail, ART_PX)} fallbacks={badge.fallbackUrls} />
    </MobileSheet>
  );
}

export function BadgeSheetBody({ facts, fallbacks }: { facts: BadgeCardFacts; fallbacks?: readonly string[] }) {
  const { title, providerLabel, description, art, website, holders, owned } = facts;
  return (
    <div className="flex flex-col items-center pb-2 pt-2 text-center">
      <BadgeArt key={art} src={art} fallbacks={fallbacks} box={ART_PX} alt={title} />
      <h2 className="mt-3 text-[18px] font-semibold leading-tight text-textPrimary [overflow-wrap:anywhere]">{title}</h2>
      {providerLabel !== title && <p className="mt-1 text-[13px] leading-[18px] text-textSecondary">{providerLabel}</p>}
      {description && <p className="mt-3 text-[14px] leading-snug text-textSecondary">{description}</p>}
      {(holders !== null || owned) && (
        <div className="mt-3 flex items-center justify-center gap-2 text-[12px]">
          {owned && (
            <span className="flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 font-medium text-accent">
              <Check size={12} strokeWidth={3} />
              You have this
            </span>
          )}
          {holders !== null && (
            <span className="tabular-nums text-textMuted">
              {holders.toLocaleString()} holder{holders === 1 ? '' : 's'}
            </span>
          )}
        </div>
      )}
      {website && (
        <button
          type="button"
          onClick={() => void openExternal(website)}
          className="mt-5 flex min-h-[44px] w-full items-center justify-center gap-1.5 rounded-xl bg-glass px-3 text-[13px] font-medium text-textPrimary"
        >
          <ExternalLink size={15} />
          Open website
        </button>
      )}
    </div>
  );
}
