// The big-art hover card for a badge, shaped like the chat emote hover card
// (centered column: art, name, source, then a quiet hint), and the art box the
// badge spotlight shares with it.
import { useState } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { badgeProviderLabel, pickBadgeArt, type BadgeRef } from '../../utils/badgeSpotlight';

const isVector = (url: string) => /\.svg(\?|#|$)/i.test(url) || url.startsWith('data:image/svg');

/**
 * Badge art centered in a fixed `box`. Small art is never blown up past twice
 * its own size (a 18 px Kick badge shows at 36 px, not 72), vector art fills
 * the box, and the box keeps its size while loading or broken so nothing
 * around it moves. Keyed by `src` at the call site so a new badge starts clean.
 */
export function BadgeArt({
  src,
  fallbacks,
  box,
  alt,
}: {
  src: string;
  fallbacks?: readonly string[];
  box: number;
  alt: string;
}) {
  const [edge, setEdge] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  const limit = edge ? Math.min(box, edge * 2) : box;
  return (
    <span className="flex shrink-0 items-center justify-center" style={{ width: box, height: box }}>
      {src && !failed && (
        <img
          src={src}
          alt={alt}
          draggable={false}
          referrerPolicy="no-referrer"
          className="object-contain drop-shadow-md transition-opacity duration-150 ease-out"
          style={{ width: limit, height: limit, opacity: edge ? 1 : 0 }}
          onLoad={(e) => {
            const img = e.currentTarget;
            const natural = Math.max(img.naturalWidth, img.naturalHeight);
            setEdge(isVector(img.currentSrc || img.src) || !natural ? box : natural);
          }}
          onError={(e) => {
            // Walk the fallbacks (larger to smaller), skipping the one that
            // just failed; an empty box beats the broken-image glyph.
            const img = e.currentTarget;
            const ladder = fallbacks ?? [];
            let step = Number(img.dataset.fb || '0');
            while (step < ladder.length && ladder[step] === img.src) step++;
            if (step < ladder.length) {
              img.dataset.fb = String(step + 1);
              img.src = ladder[step];
              return;
            }
            setFailed(true);
          }}
        />
      )}
    </span>
  );
}

/** Art edge in the hover card, the emote card's default preview step down. */
export const BADGE_HOVER_ART_PX = 72;

/**
 * Tooltip content for a badge: art at up to 72 px, its name, where it comes
 * from, and "Click for details" when it opens the spotlight. Under the
 * compact-tooltips setting (the one the emote card follows) it is the name
 * alone. Mounted only while the tooltip shows, so its store read costs chat
 * rows nothing.
 */
export function BadgeHoverCard({ badge, clickable }: { badge: BadgeRef; clickable: boolean }) {
  const compact = useAppStore((s) => !!s.settings.chat_design?.compact_emote_tooltips);
  const provider = badgeProviderLabel(badge.provider);
  const title = badge.title || provider;
  if (compact) return <>{title}</>;
  const src = pickBadgeArt(badge.art, BADGE_HOVER_ART_PX);
  return (
    <div className="flex max-w-[220px] flex-col items-center gap-1.5 py-0.5">
      <BadgeArt key={src} src={src} fallbacks={badge.fallbackUrls} box={BADGE_HOVER_ART_PX} alt={title} />
      <div className="flex flex-col items-center gap-0.5 text-center">
        <span className="text-[13px] font-bold leading-tight [overflow-wrap:anywhere]">{title}</span>
        {provider !== title && <span className="text-[10px] leading-tight text-white/60">{provider}</span>}
        {clickable && <span className="mt-0.5 text-[10px] text-white/50">Click for details</span>}
      </div>
    </div>
  );
}
