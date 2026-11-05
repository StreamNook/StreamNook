// What a member's sheet paints around its content: their Atmosphere over the
// whole sheet (or their rank's aura when they wear none), held fixed behind the
// scrolling content under the desktop card's scrim; a paint wash in the hero for
// a paint theme; and their Frame as a nine-slice border around the hero.
//
// The Atmosphere is `AtmosphereBackground` as the desktop card draws it (a
// picture Atmosphere gets the 10 px blur that component already puts on it;
// curtains none), under the desktop's 0.58 scrim and no 44 px blur. Whatever
// that component does under Battery Saver and Motion Off it does here. The
// rank aura breathes through `.sn-atm-breathe-1`, which rests under both
// (globals.css). The paint wash is held still. Nothing here adds a loop.
import React from 'react';
import { AtmosphereBackground } from '../../components/AtmosphereBackground';
import type { Atmosphere } from '../../services/atmospheres';
import type { PaintV4 } from '../../services/paintStyle';
import { computePaintStyle } from '../../services/seventvService';
import type { FrameView } from '../../types/profileView';
import { hexRgb, type AuraSpec } from './profileSheetParts';

/** The desktop card's body scrim over an Atmosphere. */
const SCRIM = 'rgba(10,10,14,0.58)';

/** Fills its positioned, clipped parent (MobileSheet's `backdrop` slot). */
export const SheetBackdrop: React.FC<{ aura: AuraSpec; atm: Atmosphere | null }> = ({ aura, atm }) => {
  if (aura.kind === 'atmosphere' && atm) {
    return (
      <>
        <AtmosphereBackground atm={atm} variant="profile" blur={!!atm.image} />
        {/* glass-exempt: the member's own look, under the desktop card's scrim. */}
        <div className="absolute inset-0" style={{ background: SCRIM }} />
      </>
    );
  }
  const rgb = aura.kind === 'aura' ? hexRgb(aura.color) : null;
  if (rgb && aura.alpha) {
    return (
      <div
        className="sn-atm-breathe-1 absolute inset-0"
        style={{
          background: `radial-gradient(ellipse 90% 55% at 50% 0%, rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${aura.alpha}), transparent 70%)`,
        }}
      />
    );
  }
  return null;
};

const WASH_MASK = 'linear-gradient(to bottom, black, transparent 80%)';

/** A paint theme's wash: the member's paint blurred into a soft glow over the
 *  hero, fading out below. Held still at 0.20 (the desktop breathes it; a
 *  breathing blurred layer costs frames on a phone). An image paint draws its
 *  first frame under Battery Saver and Motion Off (`still`). */
export const PaintWash: React.FC<{ paint: PaintV4; holdStill: boolean }> = ({ paint, holdStill }) => {
  const ps = computePaintStyle(paint, '#9146FF', 'all', holdStill);
  const base = typeof ps.backgroundColor === 'string' && !ps.backgroundColor.startsWith('var') ? ps.backgroundColor : undefined;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 overflow-hidden"
      style={{ opacity: 0.2, WebkitMaskImage: WASH_MASK, maskImage: WASH_MASK }}
    >
      <div
        className="absolute inset-0"
        style={{
          backgroundImage: ps.backgroundImage,
          backgroundColor: base,
          backgroundSize: 'cover',
          backgroundPosition: 'center',
          backgroundRepeat: 'no-repeat',
          filter: 'blur(40px) saturate(1.3)',
        }}
      />
    </div>
  );
};

/** The Frame as a nine-slice border over the hero block, as the desktop card
 *  draws it (ProfileFrame), with the numbers Rust serves. */
export const FrameOverlay: React.FC<{ frame: FrameView }> = ({ frame }) => {
  const width = `${frame.border[0]}px ${frame.border[1]}px`;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 z-[3]"
      style={{
        borderStyle: 'solid',
        borderWidth: width,
        borderImageSource: `url(${frame.url})`,
        borderImageSlice: frame.slice.join(' '),
        borderImageWidth: width,
        borderImageRepeat: 'stretch',
      }}
    />
  );
};
