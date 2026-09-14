import { useEffect, useState, useRef, useSyncExternalStore, memo } from 'react';
import type { ReactNode, MouseEvent } from 'react';
// Fraunces italic backs the tier-badge rank number below; importing it here
// (not main.tsx) keeps the axis out of every window's boot path.
import '@fontsource-variable/fraunces/wght-italic.css';
import { Tooltip } from './ui/Tooltip';
import streamNookLogo from '../assets/streamnook-logo-128.webp';
import {
  getActiveCosmeticSlug,
  getCosmeticBySlug,
  getCosmeticsVersion,
  subscribeCosmeticsVersion,
  getAtmospheresVersion,
  subscribeAtmospheresVersion,
} from '../services/supabaseService';
import { resolveCosmeticAsset } from './cosmeticAssets';
import { openProfileViewerInMain } from '../utils/openBadgesInMain';
import { useChatUserStore } from '../stores/chatUserStore';
import { AtmosphereBackground } from './AtmosphereBackground';
import { MajorCologneChrome } from './MajorCologneChrome';
import { getAtmosphere } from '../services/atmospheres';
import { MAJOR_COLOGNE_THEME_ID } from '../services/cologneEvent';

interface StreamNookBadgeProps {
  userId: string | undefined;
  userNumber: number | null;
  /** Which side of the trigger the hover popover renders on. Defaults to
   *  'top' (chat convention — popover sits above the chat row). Surfaces
   *  near the top of the viewport (e.g. ProfileSettings header) should pass
   *  'bottom' so the popover grows downward and doesn't clip the title bar.
   *  TooltipManager has a flip-on-overflow guard but it measures the
   *  popover ONCE at mount, before the cypher-reveal animation expands it,
   *  so the auto-flip can miss this. */
  side?: 'top' | 'bottom' | 'left' | 'right';
}

// Rank tiers based on signup order. The cutoffs and labels are intentionally
// kept here in one place; if these names ever change, everyone who already saw
// their old tier label will notice, so treat them as durable.
//
// CARD layout (not a tooltip pill). Each tier renders as a composed card:
//   ┌────────────────────────────┐
//   │     STREAMNOOK             │  <- wordmark, tiny tracked caps
//   │                            │
//   │   Nº   01                  │  <- number, prominent, tier-styled
//   │                            │
//   │   ─────                    │  <- thin tier-colored hairline
//   │   ETHEREAL                 │  <- tier label, tier-styled
//   └────────────────────────────┘
//
// For Ethereal (#1) only: a soft internal violet aura sits behind the content
// (radial gradient on the card bg, NOT on the label text, so it doesn't
// violate the "no glow on label" rule from past iterations). The number's
// hue-melt and the aura are the two visual signatures of tier #1 and do not
// propagate to any other tier or anywhere else in the app.
//
// All Tailwind class strings are kept as literals so JIT can scan them.
interface TierInfo {
  label: string | null;
  numberClassName: string;
  hairlineClassName: string;
  labelClassName: string;
  /** Optional internal aura overlay (Ethereal only). */
  auraClassName?: string;
}

// Shared card chassis. Liquid-glass material, the most premium surface in
// the app, with a stronger bevel than glass-button. Generous padding and
// rounded-2xl give it physical presence vs. the prior tooltip pill.
const CARD_CLASS =
  'relative inline-flex flex-col items-center px-7 py-5 rounded-2xl pointer-events-none min-w-[220px] ' +
  'bg-[rgba(14,14,18,0.92)] backdrop-blur-2xl border border-white/[0.04] ' +
  'shadow-[inset_1px_1px_0_0_rgba(255,255,255,0.14),inset_-1px_-1px_0_0_rgba(0,0,0,0.50),0_20px_50px_-15px_rgba(0,0,0,0.7),0_0_30px_-5px_rgba(0,0,0,0.4)]';

// Number typography. Fraunces Variable (italic) at a light weight gives a
// silky calligraphic display-serif feel that Satoshi can't produce. The italic
// axis is imported at the top of this file so it rides this chunk, not boot.
// `font-style: italic` is implied by the import variant; we also set it
// explicitly via Tailwind so it's visible in the JSX.
const NUMBER_BASE_CLASS =
  "font-['Fraunces_Variable'] italic font-light text-[52px] tabular-nums leading-none tracking-tight";

// Label typography. Small, tracked-out caps. Reads as classic museum-style
// nameplate subtitle.
const LABEL_BASE_CLASS =
  'text-[10px] uppercase tracking-[0.28em] font-semibold leading-none';

// Easter-egg ranks. Numbers with cultural / meme / gaming significance get a
// custom label overriding the standard tier name (Ascendant / Founder / Member /
// etc.). The number color, hairline, aura, and label STYLING all stay tied to
// the underlying tier band — only the label TEXT changes. That keeps the visual
// hierarchy intact and makes the surprise purely about the reveal moment:
// you expected "FOUNDER" and got "BLAZE IT".
//
// Keep this list deliberately small. The magic is in recognizing the number;
// enumerating every meme would dilute the discovery. If a candidate doesn't
// trigger immediate "oh!" recognition for a broad audience, leave it out.
const EASTER_EGGS: Record<number, string> = {
  42: 'The Answer',     // Hitchhiker's Guide to the Galaxy
  67: 'Six Seven',      // TikTok meme (LaMelo Ball)
  69: 'Nice',           // universal
  141: 'Going Dark',    // Task Force 141 / Call of Duty
  404: 'Not Found',     // web HTTP status
  420: 'Blaze It',      // cannabis culture
  1337: 'Leet',         // leetspeak / hacker culture
  9000: 'Over 9000',    // Dragon Ball Z
};

export const getTier = (n: number): TierInfo => {
  const base = getTierBase(n);
  const easterEgg = EASTER_EGGS[n];
  return easterEgg ? { ...base, label: easterEgg } : base;
};

export type { TierInfo };

export interface TierAccent {
  /** "r, g, b" for use inside rgba(...). */
  rgb: string;
  /** Alpha for the ambient aura gradient. */
  auraAlpha: number;
}

// The single signature color of each tier band, for theming whole surfaces
// (e.g. the public profile overlay) to the member's tier. Mirrors the bands in
// getTierBase: Ethereal violet, Mythic amber, Ascendant cyan, then near-white /
// muted for the bulk tiers (subtle, not colorless).
export const getTierAccent = (n: number): TierAccent => {
  if (n === 1) return { rgb: '139, 92, 246', auraAlpha: 0.16 }; // Ethereal violet
  if (n <= 10) return { rgb: '251, 191, 36', auraAlpha: 0.13 }; // Mythic amber
  if (n <= 100) return { rgb: '34, 211, 238', auraAlpha: 0.1 }; // Ascendant cyan
  if (n <= 250) return { rgb: '226, 232, 240', auraAlpha: 0.05 }; // Founder near-white
  return { rgb: '148, 163, 184', auraAlpha: 0.04 }; // Member muted
};

const getTierBase = (n: number): TierInfo => {
  if (n === 1) {
    return {
      label: 'Ethereal',
      numberClassName: `${NUMBER_BASE_CLASS} sn-ethereal-text`,
      hairlineClassName: 'bg-gradient-to-r from-transparent via-violet-400/50 to-transparent',
      // Italic violet, slightly lighter weight than the chip tiers. The
      // number's hue-melt and the card's internal aura carry the Ethereal
      // identity; the label stays restrained. No chip, no glow on the text.
      labelClassName: 'text-[11px] italic tracking-[0.22em] font-light text-violet-200',
      // Soft internal aura via radial violet gradient on the card bg, sitting
      // BEHIND the content. Not a glow on any text element.
      auraClassName: 'absolute inset-0 rounded-2xl bg-[radial-gradient(ellipse_at_50%_70%,rgba(139,92,246,0.18),transparent_60%)] pointer-events-none',
    };
  }
  if (n <= 10) {
    // Mythic: the second tier, only 9 people. Visually shares lineage with
    // Ethereal (italic light label, soft tier-tinted aura) to read as the
    // "inner circle," distinct from the bulk brand-color tiers below.
    return {
      label: 'Mythic',
      numberClassName: `${NUMBER_BASE_CLASS} text-amber-200`,
      hairlineClassName: 'bg-gradient-to-r from-transparent via-amber-400/45 to-transparent',
      // Italic light, same shape as Ethereal's label. The founding-tier band
      // shares this treatment so they cluster visually.
      labelClassName: 'text-[11px] italic tracking-[0.22em] font-light text-amber-200',
      // Soft warm amber aura. Lower alpha + tighter falloff than Ethereal's
      // violet so the hierarchy is clear (Ethereal still reads as MORE).
      auraClassName: 'absolute inset-0 rounded-2xl bg-[radial-gradient(ellipse_at_50%_75%,rgba(251,191,36,0.10),transparent_55%)] pointer-events-none',
    };
  }
  if (n <= 100) {
    return {
      label: 'Ascendant',
      numberClassName: `${NUMBER_BASE_CLASS} text-accent`,
      hairlineClassName: 'bg-gradient-to-r from-transparent via-cyan-400/35 to-transparent',
      labelClassName: `${LABEL_BASE_CLASS} text-accent`,
    };
  }
  if (n <= 250) {
    return {
      label: 'Founder',
      numberClassName: `${NUMBER_BASE_CLASS} text-textPrimary`,
      hairlineClassName: 'bg-gradient-to-r from-transparent via-white/20 to-transparent',
      labelClassName: `${LABEL_BASE_CLASS} text-textPrimary`,
    };
  }
  // Member: bulk community tier. Now renders a label (previously was the
  // unlabeled fallback). Muted color so it sits visually quieter than Founder.
  return {
    label: 'Member',
    numberClassName: `${NUMBER_BASE_CLASS} text-textPrimary`,
    hairlineClassName: 'bg-gradient-to-r from-transparent via-white/15 to-transparent',
    labelClassName: `${LABEL_BASE_CLASS} text-textSecondary`,
  };
};

// Mixed pool: digits (the target glyphs), half-width katakana for the matrix flavor,
// plus a few punctuation marks for visual variety while scrambling.
const SCRAMBLE_POOL =
  '0123456789' +
  'アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン' +
  '!#$%&*+';

const pickRandomGlyph = () =>
  SCRAMBLE_POOL.charAt(Math.floor(Math.random() * SCRAMBLE_POOL.length));

const SCRAMBLE_MS = 700;     // duration of the pure-cypher scramble phase
const SWAP_INTERVAL_MS = 60; // throttle for glyph swaps during scramble (~16/sec)
const MIN_SCRAMBLE_WIDTH = 3; // minimum cypher width so single-digit numbers feel substantial

interface MatrixDecodeProps {
  numberText: string;
  tier: TierInfo;
  /** When true, mount straight in the resolved state (no cypher, no fade-in).
      Used for surfaces where the user is looking at their *own* rank (e.g.
      the Profile settings panel), since the cypher reveal is a "discover
      someone else's tier" moment, not a self-check moment. */
  skipCypher?: boolean;
  /** Name of the user's active cosmetic badge (Supporter, Subscriber, etc.).
      Shown below the tier label in a smaller secondary tag so a hover reveals
      both "who is this person in the StreamNook timeline" and "which badge
      are they wearing" in one popover. Suppressed for the default cosmetic
      since the wordmark + tier label already convey that identity. */
  cosmeticName?: string | null;
}

const MatrixDecode = ({ numberText, tier, skipCypher = false, cosmeticName = null }: MatrixDecodeProps) => {
  const padWidth = Math.max(numberText.length, MIN_SCRAMBLE_WIDTH);

  // Build the initial scramble synchronously so the first paint has glyphs in it
  // rather than an empty string flicker before the RAF loop kicks in.
  const [scramble, setScramble] = useState(() => {
    let s = '';
    for (let i = 0; i < padWidth; i++) s += pickRandomGlyph();
    return s;
  });
  // When skipCypher is set, start fully resolved (no scramble, no fade-in).
  const [revealed, setRevealed] = useState(skipCypher);
  const [animateIn, setAnimateIn] = useState(skipCypher);

  const rafRef = useRef<number | null>(null);
  const revealTimerRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    if (skipCypher) return; // no animation work needed
    const startTime = performance.now();
    let lastSwapTime = startTime;
    let current: string[] = [];
    for (let i = 0; i < padWidth; i++) current.push(pickRandomGlyph());

    const tick = () => {
      const now = performance.now();
      const elapsed = now - startTime;

      if (now - lastSwapTime >= SWAP_INTERVAL_MS) {
        current = current.map(() => pickRandomGlyph());
        lastSwapTime = now;
        setScramble(current.join(''));
      }

      if (elapsed < SCRAMBLE_MS) {
        rafRef.current = requestAnimationFrame(tick);
      }
    };
    rafRef.current = requestAnimationFrame(tick);

    revealTimerRef.current = setTimeout(() => {
      setRevealed(true);
      // Defer flipping animateIn by two frames so the resolved layout mounts
      // at its initial state (opacity-0, translate-y-1) before transitioning
      // to its final state. Single rAF is sometimes not enough; paint can
      // batch the two state updates and skip the from-state.
      requestAnimationFrame(() => {
        requestAnimationFrame(() => setAnimateIn(true));
      });
    }, SCRAMBLE_MS);

    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      if (revealTimerRef.current !== null) clearTimeout(revealTimerRef.current);
    };
  }, [padWidth, skipCypher]);

  // Card composition. During the cypher phase the card houses only the
  // number row, so the cypher is the visual center. On reveal, the chrome
  // around it (wordmark above; hairline / tier label / cosmetic block below)
  // expands into place via a max-height + opacity transition. Tooltip
  // re-positions because its anchor calculation runs against the trigger
  // rect, so the cypher stays vertically near where it was during scramble.
  const chromeOpacity = `transition-opacity duration-300 ease-out ${
    animateIn ? 'opacity-100' : 'opacity-0'
  }`;
  const collapsibleWrapper = `overflow-hidden transition-[max-height,opacity] duration-500 ease-out`;
  const aboveCollapsed = animateIn ? 'max-h-12 opacity-100' : 'max-h-0 opacity-0';
  const belowCollapsed = animateIn ? 'max-h-48 opacity-100' : 'max-h-0 opacity-0';

  return (
    <>
      {tier.auraClassName && <div className={tier.auraClassName} />}
      <div className="relative flex flex-col items-center w-full">
        {/* Above-cypher chrome: wordmark. Collapsed during scramble so the
            cypher reads as the popover's vertical center. */}
        <div className={`${collapsibleWrapper} ${aboveCollapsed} flex flex-col items-center`}>
          <div className="text-[9px] uppercase tracking-[0.36em] font-medium text-white/35 mb-4">
            StreamNook
          </div>
        </div>

        {/* Number row: leading Nº prefix + number, with a phantom Nº mirror on
            the right (visibility: hidden) so the flex row is symmetric in both
            phases. The visible Nº and phantom Nº have identical font metrics
            ⇒ identical widths ⇒ the scramble / resolved number always sits at
            the geometric center of the row, regardless of whether the leading
            Nº is at opacity-0 (cypher) or opacity-1 (reveal). */}
        <div className="flex items-baseline justify-center gap-2">
          <span className={`text-[11px] text-white/30 font-light leading-none ${chromeOpacity}`}>
            Nº
          </span>
          {!revealed ? (
            <span className="font-mono text-[52px] font-extralight text-textPrimary tabular-nums leading-none tracking-tight">
              {scramble}
            </span>
          ) : (
            <span className={tier.numberClassName}>{numberText}</span>
          )}
          <span className="text-[11px] font-light leading-none invisible" aria-hidden="true">
            Nº
          </span>
        </div>

        {/* Below-cypher chrome: hairline + tier label + cosmetic block. Also
            collapsed during scramble, expands on reveal. */}
        <div className={`${collapsibleWrapper} ${belowCollapsed} flex flex-col items-center`}>
          <div className={`w-16 h-px ${tier.hairlineClassName} mt-3.5 mb-2.5`} />
          {tier.label && <div className={tier.labelClassName}>{tier.label}</div>}
          {cosmeticName && (
            <div className="mt-3 flex flex-col items-center">
              <div className="text-[9px] uppercase tracking-[0.36em] font-medium text-white/35 mb-1.5">
                Badge
              </div>
              <div className="text-[11px] uppercase tracking-[0.22em] font-light text-white/80">
                {cosmeticName}
              </div>
            </div>
          )}
        </div>
      </div>
    </>
  );
};

// Standalone tier card with chassis + aura + composed content. Used both as the
// Tooltip content for the chat-badge hover (via StreamNookBadge) and as an
// inline element in any "your identity" surface (e.g. ProfileSettings). When
// inline, place it directly; when in a tooltip, pass CARD_CLASS as the
// tooltip's containerClassName so the tooltip bubble itself becomes the card
// chassis (avoids double-wrapping).
export const StreamNookTierCard = memo(function StreamNookTierCard({
  userNumber,
  skipCypher = false,
  cosmeticName = null,
}: { userNumber: number; skipCypher?: boolean; cosmeticName?: string | null }) {
  const tier = getTier(userNumber);
  return (
    <div className={CARD_CLASS}>
      {tier.auraClassName && <div className={tier.auraClassName} />}
      <MatrixDecode numberText={String(userNumber)} tier={tier} skipCypher={skipCypher} cosmeticName={cosmeticName} />
    </div>
  );
});

/**
 * Sync read for the active-cosmetic asset for a given Twitch user.
 *
 * Returns the resolved Vite asset URL, or null if the user has no active
 * cosmetic / the cosmetics registry has not loaded yet / the slug points at
 * an asset we don't bundle. Wired to useSyncExternalStore at the call site
 * so it re-renders when the user's selection changes (locally or via the
 * realtime subscription).
 */
const useActiveCosmeticAsset = (userId: string | undefined): string | null => {
  useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion, getCosmeticsVersion);
  if (!userId) return null;
  const slug = getActiveCosmeticSlug(userId);
  if (!slug) return null;
  // Bundled cosmetics win (the original gold trio, fingerprinted by Vite); cloud
  // cosmetics fall back to the catalog's R2 asset_path, so a new badge ships as a
  // DB row + an upload with no desktop release (same model as atmospheres).
  // chatSize: this component renders at 24px everywhere it mounts.
  return resolveCosmeticAsset(
    { slug, asset_path: getCosmeticBySlug(slug)?.asset_path },
    { chatSize: true },
  );
};

export const StreamNookBadge = memo(function StreamNookBadge({
  userId,
  userNumber,
  side = 'top',
}: StreamNookBadgeProps) {
  // Click opens this member's public StreamNook profile in the draggable viewer
  // overlay. Routed via openProfileViewerInMain so it also works from the
  // profile-card / MultiChat popout windows (their own store doesn't mount the
  // viewer). stopPropagation so the chat row's own click handler (which opens
  // the Twitch UserProfileCard) doesn't also fire.
  const handleClick = (e: MouseEvent) => {
    e.stopPropagation();
    if (userId) openProfileViewerInMain(userId);
  };

  const cosmeticAsset = useActiveCosmeticAsset(userId);
  const cosmeticSlug = getActiveCosmeticSlug(userId);
  const cosmetic = cosmeticSlug ? getCosmeticBySlug(cosmeticSlug) : null;
  const cosmeticName = cosmetic?.name ?? null;

  // The member's StreamNook Atmosphere, if chat has resolved it for this user, so
  // the cypher card adopts their profile theme. Reads the already-resolved value
  // (no per-badge fetch); a primitive selector means this only re-renders when
  // THIS user's atmosphere changes.
  // Re-render once the atmosphere catalog has loaded (or changes) so the
  // getAtmosphere lookup below resolves to the real definition.
  useSyncExternalStore(subscribeAtmospheresVersion, getAtmospheresVersion, getAtmospheresVersion);
  const atmosphereId = useChatUserStore((s) => (userId ? s.users.get(userId)?.atmosphereId ?? null : null));
  const atmosphere = atmosphereId ? getAtmosphere(atmosphereId) : null;
  // CS2 Major Cologne cosmetics this member applied (themes the hover card too).
  // The chrome asset URLs come from the Cologne atmosphere row (R2).
  const cologne = useChatUserStore((s) => (userId ? s.users.get(userId)?.cologne ?? null : null));
  const cologneAtm = cologne ? getAtmosphere(MAJOR_COLOGNE_THEME_ID) : null;

  // If the registry lookup somehow missed (shouldn't happen given isSN was true),
  // fall back to the plain label so we never render a broken animation.
  let tooltipContent: ReactNode;
  let containerClassName: string | undefined;
  if (userNumber != null) {
    const tier = getTier(userNumber);
    tooltipContent = (
      <>
        {cologne && cologneAtm ? (
          <div className="absolute inset-0 overflow-hidden rounded-2xl">
            {/* Cologne theme: the background wash (+ coin if they enabled it). The
                gold frame is a chat-row border and is omitted here so it doesn't
                fight the card's own rounded chassis. */}
            <MajorCologneChrome
              textureUrl={cologneAtm.chromeTexture ?? ''}
              coinUrl={cologneAtm.chromeCoin}
              coin={cologne.coin}
            />
          </div>
        ) : atmosphere ? (
          <div className="absolute inset-0 overflow-hidden rounded-2xl">
            {/* Frosted so the badge card's number + label stay readable over a
                busy image atmosphere (the big profile stays sharp). */}
            <AtmosphereBackground atm={atmosphere} variant="profile" blur />
          </div>
        ) : null}
        {tier.auraClassName && <div className={tier.auraClassName} />}
        <MatrixDecode numberText={String(userNumber)} tier={tier} cosmeticName={cosmeticName} />
        <div className="relative mt-3 text-[8px] font-medium uppercase tracking-[0.3em] text-white/35">
          Click to open profile
        </div>
      </>
    );
    containerClassName = CARD_CLASS;
  } else {
    tooltipContent = 'StreamNook user';
  }

  const src = cosmeticAsset ?? streamNookLogo;
  const alt = cosmeticName ? `StreamNook ${cosmeticName}` : 'StreamNook';

  return (
    <Tooltip
      content={tooltipContent}
      side={side}
      delay={120}
      containerClassName={containerClassName}
    >
      <img
        src={src}
        alt={alt}
        loading="lazy"
        className="w-6 h-6 inline-block object-contain cursor-pointer hover:scale-110 transition-transform"
        draggable={false}
        onClick={handleClick}
      />
    </Tooltip>
  );
});
