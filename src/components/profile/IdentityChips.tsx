// The identity chips beside a person's name: their worn 7TV paint, their
// StreamNook atmosphere, and a link to their 7TV profile. One set, used by the
// plain user card and a member's card, so the two always match.

import { computePaintStyle } from '../../services/seventvService';
import type { Atmosphere } from '../../services/atmospheres';
import { openBadgesOnStreamNookInMain, openBadgesWithPaintInMain } from '../../utils/openBadgesInMain';
import { Logger } from '../../utils/logger';
import { SevenTVLogo } from '../emotesets/SevenTVLogo';
import { Tooltip } from '../ui/Tooltip';

type Side = 'top' | 'bottom';

/** The glass chip every identity chip shares: a soft top highlight and bottom
 *  shade, so a paint or atmosphere fill reads as a surface, not a sticker. */
const CHIP =
  'relative inline-flex flex-shrink-0 items-center gap-1.5 overflow-hidden rounded-md border border-transparent px-2 py-0.5 text-[11px] font-bold leading-[1.35] shadow-[inset_1px_1px_0_0_rgba(255,255,255,0.10),inset_-1px_-1px_0_0_rgba(0,0,0,0.18)] transition-all hover:ring-1 hover:ring-accent/50 cursor-pointer';

/** The worn 7TV paint, drawn as the chip's fill with its name knocked out. */
export function PaintChip({
  paint,
  color,
  side = 'top',
}: {
  paint: { id: string; name: string } & Record<string, unknown>;
  color: string;
  side?: Side;
}) {
  const style = computePaintStyle(paint as never, color);
  return (
    <Tooltip content={`7TV paint: ${paint.name}`} side={side}>
      <button
        onClick={(e) => {
          e.stopPropagation();
          openBadgesWithPaintInMain(paint.id);
        }}
        className={CHIP}
        style={{ ...style, WebkitBackgroundClip: 'padding-box', backgroundClip: 'padding-box' }}
      >
        <span
          style={{
            ...style,
            filter: 'invert(1) contrast(1.5)',
            WebkitBackgroundClip: 'text',
            backgroundClip: 'text',
          }}
        >
          {paint.name}
        </span>
      </button>
    </Tooltip>
  );
}

/** The applied StreamNook atmosphere, filled with its own swatch. */
export function AtmosphereChip({ atmosphere, side = 'top' }: { atmosphere: Atmosphere; side?: Side }) {
  return (
    <Tooltip content={`Atmosphere: ${atmosphere.name}`} side={side}>
      <button
        onClick={(e) => {
          e.stopPropagation();
          openBadgesOnStreamNookInMain();
        }}
        className={`${CHIP} text-white`}
        style={{
          background: atmosphere.swatch,
          // Rimmed in the atmosphere's own accent, so a dark one still reads.
          boxShadow: `inset 0 0 0 1px rgba(${atmosphere.accent}, 0.65), inset 0 1px 0 0 rgba(255, 255, 255, 0.18), 0 0 10px -3px rgba(${atmosphere.accent}, 0.7)`,
        }}
      >
        <span
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,rgba(255,255,255,0.14),transparent_70%)]"
        />
        <span className="relative [text-shadow:0_1px_2px_rgba(0,0,0,0.7)]">{atmosphere.name}</span>
      </button>
    </Tooltip>
  );
}

/** Opens the person's 7TV profile, labeled with the 7TV logo. */
export function SevenTvProfileButton({ seventvUserId, side = 'top' }: { seventvUserId: string; side?: Side }) {
  return (
    <Tooltip content="Open 7TV profile" side={side}>
      <button
        onClick={async (e) => {
          e.stopPropagation();
          try {
            const { open } = await import('@tauri-apps/plugin-shell');
            await open(`https://7tv.app/users/${seventvUserId}`);
          } catch (err) {
            Logger.error('Failed to open 7TV profile:', err);
          }
        }}
        className={`${CHIP} bg-white/5 px-2.5`}
        aria-label="Open 7TV profile"
      >
        {/* 7TV brand blue is deliberately not tokenized: brand marks stay the
            same in every theme. */}
        <SevenTVLogo className="h-3 w-auto text-[#29b6f6]" />
      </button>
    </Tooltip>
  );
}
