import type { Atmosphere } from '../services/atmospheres';
import { MajorCologneChrome } from './MajorCologneChrome';
import { parseCologneTheme } from '../services/cologneEvent';
import { AtmosphereChatWash } from './overlay/AtmosphereChatWash';

// Renders a StreamNook Atmosphere. The motion is pure CSS (the `.sn-aurora-*`
// classes in globals.css), so it runs on the compositor off the main thread and
// stays cheap even when many chat messages each render one. Both variants use the
// SAME animation, so a member's chat wash matches their profile backdrop.
//   - 'profile' = full-panel backdrop (opaque base + drifting curtains).
//   - 'chat'    = a translucent, readability-masked wash behind a message row
//                 (no base, faded out toward the text) + a thin accent bar.

// The curtains slide 320px (sn-aurora-1) and 240px (sn-aurora-2) per loop, so
// they need at least that much overhang past each side or a strip of bare
// base shows near the end of every loop and snaps back at the restart. 80%
// alone falls short on cards under about 270px wide (the member hover card).
const CURTAIN_OVERHANG = { left: 'min(-80%, -360px)', right: 'min(-80%, -360px)' } as const;

export const AtmosphereBackground = ({
  atm,
  variant,
  blur = false,
}: {
  atm: Atmosphere;
  variant: 'profile' | 'chat';
  // Soften the animated image backdrop with a frosted blur so overlaid text
  // stays readable. Opt-in for profile backdrops: the big profile reads fine
  // sharp, but the small badge-hover card wants it. (Chat rows always blur,
  // since text sits directly on the wash.)
  blur?: boolean;
}) => {
  // The 'cologne-chrome' render kind is custom chrome (animated glass texture +
  // coin + frame) drawn from the def's R2 asset URLs, not a gradient, so render it
  // directly wherever an atmosphere would paint. Which add-ons show is parsed off
  // the carried theme id. Live chat rows go through MajorCologneChrome on their
  // own; this branch serves the profile backdrop and the picker's chat preview.
  if (atm.kind === 'cologne-chrome') {
    const c = parseCologneTheme(atm.id);
    return (
      <MajorCologneChrome
        textureUrl={atm.chromeTexture ?? ''}
        coinUrl={atm.chromeCoin}
        frameUrl={atm.chromeFrame}
        coin={!!c?.coin}
        // The frame is a square chat-row border; the rounded profile panel would
        // clash with it, so it shows only on the chat wash. Background + coin
        // still theme the profile.
        frame={variant === 'chat' && !!c?.frame}
      />
    );
  }
  if (variant === 'chat') {
    return <AtmosphereChatWash atm={atm} />;
  }
  // Image-backed atmosphere (e.g. an animated webp): the image IS the whole
  // profile backdrop and carries its own motion, so there are no CSS curtains.
  if (atm.image) {
    // When blurred, scale up a touch so the blur's soft edge fade is pushed
    // past the clip instead of revealing a fuzzy border.
    const imgFilter = blur ? 'blur(10px)' : undefined;
    const blurScale = blur ? ' scale(1.12)' : '';
    return (
      <div
        className="pointer-events-none absolute -inset-10 overflow-hidden"
        style={{ backgroundColor: atm.baseColor }}
      >
        {atm.imageProfilePortrait ? (
          // Render the landscape image rotated 90deg so it reads portrait in the
          // tall profile panel. Container-query units size the rotated layer to
          // the panel's SWAPPED dimensions (width <- panel height, height <-
          // panel width), so it still covers edge-to-edge after the rotation
          // with no gaps, on any panel aspect.
          <div className="absolute inset-0 overflow-hidden" style={{ containerType: 'size' }}>
            <div
              className="absolute left-1/2 top-1/2"
              style={{
                width: '100cqh',
                height: '100cqw',
                transform: `translate(-50%, -50%) rotate(90deg)${blurScale}`,
                backgroundImage: `url(${atm.image})`,
                backgroundSize: 'cover',
                backgroundPosition: 'center',
                filter: imgFilter,
              }}
            />
          </div>
        ) : (
          <div
            className="absolute inset-0"
            style={{
              backgroundImage: `url(${atm.image})`,
              backgroundSize: 'cover',
              backgroundPosition: 'center',
              filter: imgFilter,
              transform: blur ? 'scale(1.12)' : undefined,
            }}
          />
        )}
      </div>
    );
  }
  return (
    <div
      className="pointer-events-none absolute -inset-10 overflow-hidden"
      style={{ backgroundColor: atm.baseColor, backgroundImage: atm.baseLayers }}
    >
      <div
        className="sn-aurora-1 absolute -inset-[80%]"
        style={{ backgroundImage: atm.layers, backgroundRepeat: 'repeat', ...CURTAIN_OVERHANG }}
      />
      {atm.layers2 && (
        <div
          className="sn-aurora-2 absolute -inset-[80%]"
          style={{ backgroundImage: atm.layers2, backgroundRepeat: 'repeat', ...CURTAIN_OVERHANG }}
        />
      )}
    </div>
  );
};

// The chat-row wash lives in the shared `overlay/AtmosphereChatWash` component so
// the in-app chat and the hosted overlay render it identically (see that file).
