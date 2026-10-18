// The pieces every identity surface shares (your You card, the Edit profile
// preview, a chatter's profile sheet), so the three keep one shape: the
// Atmosphere as a band across the top, the picture cut into the band's lower
// edge with the StreamNook badge on it as a medal, and the name below the art,
// never on it. The iPhone draws the same shape from You/IdentityViews.swift.
import React, { useEffect, useRef, useState } from 'react';
import { Check } from 'phosphor-react';
import { ProviderMark } from '../../components/ProviderLogo';
import { SevenTVLogo } from '../../components/emotesets/SevenTVLogo';
import { FallbackImage } from '../../components/FallbackImage';
import { AtmosphereBackground } from '../../components/AtmosphereBackground';
import { computePaintStyle } from '../../services/seventvService';
import { getAtmosphere } from '../../services/atmospheres';
import type { PaintV4 } from '../../services/paintStyle';
import type { AvatarPick } from '../../types/profileView';
import streamnookMark from '../../assets/streamnook-logo-128.webp';
import { SEVENTV_BLUE, nameColorOr, type Brand } from './identity';

/**
 * A brand's real mark at `size` px. 7TV's wordmark is wider than tall, so
 * `size` is its height and it runs wider; the others are square.
 */
export const BrandMark: React.FC<{ brand: Brand; size: number; className?: string; color?: string }> = ({
  brand,
  size,
  className = '',
  color,
}) => {
  if (brand === 'seventv') {
    // The logo takes a class only, so the wrapper carries the height and the
    // svg fills it, its width following the wordmark's own proportions.
    return (
      <span
        className={`inline-flex shrink-0 items-center ${className}`}
        style={{ height: size, color: color ?? SEVENTV_BLUE }}
        aria-hidden="true"
      >
        <SevenTVLogo className="h-full w-auto" title="" />
      </span>
    );
  }
  if (brand === 'streamnook') {
    return (
      <img
        src={streamnookMark}
        alt=""
        draggable={false}
        className={`shrink-0 object-contain ${className}`}
        style={{ width: size, height: size }}
      />
    );
  }
  return <ProviderMark provider={brand} size={size} className={className} />;
};

/**
 * A member's Atmosphere as a band: its swatch, the full-strength art the picker
 * shows (the chat row's wash is built to sit dim behind text and reads near
 * black as a banner). Without one, a quiet wash of `tint` so the card keeps
 * its shape.
 */
export const AtmosphereBand: React.FC<{
  swatch?: string | null;
  tint: string;
  height: number;
  /** The card's corner radius, so the band's top corners follow it. */
  radius: number;
}> = ({ swatch, tint, height, radius }) => (
  <div
    aria-hidden="true"
    className="relative overflow-hidden pointer-events-none bg-tertiary"
    style={{ height, borderTopLeftRadius: radius, borderTopRightRadius: radius }}
  >
    {swatch ? (
      <div className="absolute inset-0" style={{ background: swatch }} />
    ) : (
      <div
        className="absolute inset-0"
        style={{
          backgroundImage: `radial-gradient(280px circle at 0% 0%, color-mix(in srgb, ${tint} 42%, transparent), transparent), radial-gradient(240px circle at 100% 100%, color-mix(in srgb, ${tint} 18%, transparent), transparent)`,
        }}
      />
    )}
  </div>
);

/**
 * A picture set into the card's edge: a ring of the card's own surface around
 * it so it reads as cut out of the band, the StreamNook medal at its lower
 * right. It shows the one picture others see in chat (the 7TV one when there
 * is one, else the platform's), never one over the other; when it is the 7TV
 * one, 7TV's mark sits at its lower left. With `explains`, a tap says which
 * picture shows where.
 */
export const CutoutAvatar: React.FC<{
  platformUrl?: string | null;
  picture?: AvatarPick | null;
  size: number;
  medal?: string | null;
  medalLabel?: string;
  /** A picture just picked, shown before Twitch serves it. */
  localUrl?: string | null;
  explains?: boolean;
}> = ({ platformUrl, picture, size, medal, medalLabel = '', localUrl, explains = false }) => {
  // The 7TV picture that failed every rendition; the platform one takes over.
  const [failedFor, setFailedFor] = useState<string | null>(null);
  const [explaining, setExplaining] = useState(false);
  const host = useRef<HTMLDivElement>(null);
  const ring = Math.max(3, Math.round(size * 0.05));
  const showsSevenTv = !localUrl && !!picture && failedFor !== picture.still;

  useEffect(() => {
    if (!explaining) return;
    const close = (e: PointerEvent) => {
      if (!host.current?.contains(e.target as Node)) setExplaining(false);
    };
    document.addEventListener('pointerdown', close, true);
    return () => document.removeEventListener('pointerdown', close, true);
  }, [explaining]);

  let face: React.ReactNode;
  if (localUrl) {
    face = <img src={localUrl} alt="" draggable={false} className="w-full h-full object-cover" />;
  } else if (showsSevenTv && picture) {
    face = (
      <FallbackImage
        key={picture.still}
        src={picture.animated ?? picture.still}
        fallbackUrls={picture.animated ? [picture.still] : []}
        alt=""
        draggable={false}
        className="w-full h-full object-cover"
        onAllFailed={() => setFailedFor(picture.still)}
      />
    );
  } else if (platformUrl) {
    face = <img src={platformUrl} alt="" draggable={false} className="w-full h-full object-cover" />;
  } else {
    face = null;
  }

  return (
    <div ref={host} className="relative" style={{ width: size + ring * 2, height: size + ring * 2 }}>
      <button
        type="button"
        onClick={explains ? () => setExplaining((v) => !v) : undefined}
        disabled={!explains}
        aria-label={explains ? 'Which picture shows where' : undefined}
        aria-hidden={explains ? undefined : true}
        tabIndex={explains ? 0 : -1}
        // Solid, not the theme's translucent tints: the ring is the card's own
        // fill laid on the page, so it reads as a cut in the card, and the disc
        // under the picture is the floor the tab-bar picture sits on. With the
        // tints the banner showed through both, and a 7TV picture with
        // transparent parts floated over the art.
        className="block rounded-full"
        style={{
          padding: ring,
          boxShadow: '0 4px 8px rgba(0,0,0,0.35)',
          background: 'linear-gradient(var(--color-background-secondary), var(--color-background-secondary)), var(--color-background)',
        }}
      >
        <span
          className="block rounded-full overflow-hidden ring-1 ring-white/10"
          style={{
            width: size,
            height: size,
            background: 'linear-gradient(var(--color-surface), var(--color-surface)), var(--color-background-tertiary)',
          }}
        >
          {face}
        </span>
      </button>
      {medal && (
        // On a disc of the card's floor, so it reads as a medal, not a mark
        // scratched on the photo.
        <span
          className="absolute flex items-center justify-center rounded-full bg-tertiary pointer-events-none"
          style={{
            width: size * 0.32,
            height: size * 0.32,
            right: ring * 0.6,
            bottom: ring * 0.6,
            boxShadow: 'inset 0 0 0 0.5px rgba(0,0,0,0.4)',
          }}
          role="img"
          aria-label={medalLabel}
        >
          <img src={medal} alt="" draggable={false} className="object-contain" style={{ width: size * 0.24, height: size * 0.24 }} />
        </span>
      )}
      {showsSevenTv && (
        <span
          className="absolute flex items-center justify-center rounded-full bg-tertiary pointer-events-none"
          style={{
            width: size * 0.26,
            height: size * 0.26,
            left: ring * 0.6,
            bottom: ring * 0.6,
            boxShadow: 'inset 0 0 0 0.5px rgba(0,0,0,0.4)',
          }}
          role="img"
          aria-label="7TV picture"
        >
          <BrandMark brand="seventv" size={Math.round(size * 0.11)} />
        </span>
      )}
      {explaining && <PictureNote sevenTv={showsSevenTv} />}
    </div>
  );
};

/** Which of your two pictures shows where, as the avatar's tap explains it. */
const PictureNote: React.FC<{ sevenTv: boolean }> = ({ sevenTv }) => (
  <div
    role="note"
    className="absolute left-0 top-full mt-2 z-30 w-[290px] max-w-[calc(100vw-32px)] glass-modal rounded-xl p-4 flex flex-col gap-2.5"
  >
    <NoteLine brand="seventv">
      {sevenTv
        ? 'Your 7TV picture. StreamNook and 7TV viewers see it beside your name in chat.'
        : 'Set a 7TV picture and StreamNook and 7TV viewers see it in chat instead.'}
    </NoteLine>
    <NoteLine brand="twitch">
      Your Twitch picture shows on Twitch itself, and to everyone where you have no 7TV picture.
    </NoteLine>
    <div className="text-[12px] text-textMuted">Change either in Edit profile.</div>
  </div>
);

const NoteLine: React.FC<{ brand: Brand; children: React.ReactNode }> = ({ brand, children }) => (
  <div className="flex items-start gap-2.5">
    <span className="w-[18px] shrink-0 flex justify-center pt-[3px]">
      <BrandMark brand={brand} size={brand === 'seventv' ? 10 : 13} />
    </span>
    <span className="text-[14px] leading-snug text-textPrimary">{children}</span>
  </div>
);

/**
 * A name in its 7TV paint, else in `color`. Wraps rather than clamps: a clamp
 * needs `display: -webkit-box`, which fights the `background-clip: text` a
 * paint is drawn with.
 */
export const PaintedName: React.FC<{
  text: string;
  paint?: PaintV4 | null;
  color: string;
  className?: string;
  style?: React.CSSProperties;
}> = ({ text, paint, color, className = '', style }) => (
  <span className={className} style={{ ...(paint ? computePaintStyle(paint, color, 'all') : { color }), ...style }}>
    {text}
  </span>
);

/** A section's label: the brand's mark where a brand owns the section, then
 *  the words, then a quieter detail (what is worn, a count). */
export const SectionHeader: React.FC<{
  title: string;
  brand?: Brand | null;
  detail?: string | null;
  trailing?: React.ReactNode;
  className?: string;
}> = ({ title, brand, detail, trailing, className = '' }) => (
  <div className={`flex items-center gap-1.5 min-w-0 ${className}`} role="heading" aria-level={3}>
    {brand && <BrandMark brand={brand} size={brand === 'seventv' ? 10 : 12} />}
    <span className="text-[12px] font-semibold uppercase tracking-[0.05em] text-textMuted shrink-0">{title}</span>
    {detail && <span className="text-[12px] text-textMuted truncate min-w-0">{detail}</span>}
    <span className="flex-1" />
    {trailing}
  </div>
);

/** A worn badge at 18 px; one whose every URL fails takes itself out. */
export const WornBadge: React.FC<{ url: string; fallbacks?: string[]; label?: string }> = ({
  url,
  fallbacks,
  label,
}) => (
  <FallbackImage
    src={url}
    fallbackUrls={fallbacks}
    alt={label ?? ''}
    title={label}
    draggable={false}
    className="w-[18px] h-[18px] object-contain shrink-0"
  />
);

/**
 * A chat row of yours as others will see it, live as you pick: your badges in
 * chat's order (Twitch, 7TV, StreamNook last), your name in your paint or your
 * Twitch color, a line of chat, and your Atmosphere's wash behind it (the same
 * wash chat draws, with its frost when it asks for one) while it shows in chat.
 */
export const ProfilePreviewRow: React.FC<{
  name: string;
  paint?: PaintV4 | null;
  colorHex?: string | null;
  badges: string[];
  /** The Atmosphere shown in chat, or null while none is. */
  atmosphereId?: string | null;
}> = ({ name, paint, colorHex, badges, atmosphereId }) => {
  const atm = atmosphereId ? getAtmosphere(atmosphereId) : null;
  const frost = !!atm?.chatFrost;
  return (
    <div
      className="relative overflow-hidden rounded-xl px-3 py-2.5"
      style={atm ? undefined : { backgroundColor: 'rgba(0,0,0,0.28)' }}
      aria-label={`Preview: ${name}: that play was clean`}
      role="img"
    >
      {atm && <AtmosphereBackground atm={atm} variant="chat" />}
      <div
        className={`relative flex items-center gap-1 min-w-0 max-w-full ${frost ? 'w-fit rounded-md px-1.5 py-0.5' : ''}`}
        style={frost ? { backgroundColor: 'rgba(5,6,13,0.66)' } : undefined}
        aria-hidden="true"
      >
        {badges.map((url, i) => (
          <WornBadge key={`${url}-${i}`} url={url} />
        ))}
        <span className={`flex items-baseline min-w-0 ${badges.length ? 'ml-0.5' : ''}`}>
          <PaintedName
            text={name}
            paint={paint}
            color={nameColorOr(colorHex)}
            className="text-[14px] font-bold whitespace-nowrap shrink-0"
          />
          <span className="text-[14px] font-light text-textPrimary truncate">: that play was clean</span>
        </span>
      </div>
    </div>
  );
};

/** A brand's mark in a round tile of its own color: the picture of an account
 *  row, or a connect prompt, that has no avatar. */
export const BrandTile: React.FC<{ brand: Brand; size?: number; color: string }> = ({ brand, size = 44, color }) => (
  <div
    className="rounded-full shrink-0 flex items-center justify-center"
    style={{
      width: size,
      height: size,
      backgroundColor: `color-mix(in srgb, ${color} 14%, transparent)`,
      boxShadow: `inset 0 0 0 1px color-mix(in srgb, ${color} 22%, transparent)`,
    }}
  >
    <BrandMark brand={brand} size={Math.round(size * (brand === 'seventv' ? 0.3 : 0.42))} />
  </div>
);

/** The worn mark on a picker tile: an accent ring around it and a check on its
 *  corner. Lay it inside a `relative` tile with the tile's own radius. */
export const WornMarks: React.FC<{ radius: number }> = ({ radius }) => (
  <>
    <span
      aria-hidden="true"
      className="absolute inset-0 pointer-events-none border-2 border-accent"
      style={{ borderRadius: radius }}
    />
    <span
      aria-hidden="true"
      className="absolute -top-[5px] -right-[5px] w-[18px] h-[18px] rounded-full bg-accent flex items-center justify-center pointer-events-none"
    >
      <Check size={10} weight="bold" className="text-background" />
    </span>
  </>
);
