// Renders a source platform's real brand mark (Twitch glitch / Kick), bundled as
// colored SVGs. Falls back to a brand-color dot for providers whose logo isn't
// bundled yet. Used on MultiChat tabs, per-column headers, and mod-panel cells.

import { Tooltip } from './ui/Tooltip';
import twitchLogo from '../assets/provider-logos/twitch.svg?url';
import kickLogo from '../assets/provider-logos/kick.svg?url';
import youtubeLogo from '../assets/provider-logos/youtube.svg?url';
import tiktokLogo from '../assets/provider-logos/tiktok.svg?url';
import { PROVIDERS, type ProviderId } from '../types/providers';

const LOGOS: Partial<Record<ProviderId, string>> = {
  twitch: twitchLogo,
  kick: kickLogo,
  youtube: youtubeLogo,
  tiktok: tiktokLogo,
};

/** The bundled brand-logo URL for a provider, or undefined if not bundled. */
export function providerLogo(provider: ProviderId): string | undefined {
  return LOGOS[provider];
}

/** Optical correction, not a layout fudge. Every logo ships on a 24x24 canvas
 *  but they do not fill it equally: the Twitch and Kick glyphs run the full
 *  height of theirs, while YouTube's badge is a wide plate covering about 71%
 *  of its box with the rest empty. Drawn at the same nominal size, YouTube
 *  reads noticeably smaller and lighter than the others. Applied as a transform
 *  so the slot still measures `size` and the overhang lands in the gap. */
const OPTICAL_SCALE: Partial<Record<ProviderId, number>> = { youtube: 1.18 };

/** Second optical correction, for BOTTOM-ALIGNED contexts only (a mark sitting
 *  in an `items-end` row next to a text baseline, like the stream tile's
 *  platform mark). Twitch and Kick ink runs to the bottom edge of the 24-box,
 *  but YouTube's plate is vertically CENTERED (3.545 units empty below it), so
 *  box-bottom alignment leaves its ink floating ~15% high. After the 1.18
 *  optical scale about center, the residual gap is 3.545/24 - (16.91*0.18/2)/24
 *  = ~8.4% of the box; translating by it plants the ink on the shared bottom.
 *  Centered contexts (tab strips, login buttons) must NOT apply this: there
 *  the glyph centers already align. */
const OPTICAL_INK_BOTTOM: Partial<Record<ProviderId, string>> = { youtube: '8.4%' };

type OpticalAlign = 'center' | 'ink-bottom';

function opticalTransform(provider: ProviderId, align: OpticalAlign): string | undefined {
  const scale = OPTICAL_SCALE[provider];
  const drop = align === 'ink-bottom' ? OPTICAL_INK_BOTTOM[provider] : undefined;
  const parts = [drop ? `translateY(${drop})` : null, scale ? `scale(${scale})` : null].filter(Boolean);
  return parts.length ? parts.join(' ') : undefined;
}

/**
 * The brand mark on its own: no tooltip, no wrapper chrome, optically sized.
 *
 * `ProviderLogo` below wraps itself in a Tooltip, which is right for a bare
 * mark standing in for a name (a MultiChat tab) and wrong everywhere the name
 * is already written next to it. Inside a button that says "Login with Kick",
 * a tooltip reading "Kick" is noise.
 */
export function ProviderMark({
  provider,
  size = 16,
  className = '',
  opticalAlign = 'center',
}: {
  provider: ProviderId;
  size?: number;
  className?: string;
  opticalAlign?: OpticalAlign;
}) {
  const src = providerLogo(provider);
  const transform = opticalTransform(provider, opticalAlign);
  return (
    <span
      className={`flex flex-shrink-0 items-center justify-center leading-none ${className}`}
      style={{ width: size, height: size }}
    >
      {src ? (
        <img
          src={src}
          alt=""
          draggable={false}
          style={{ width: size, height: size, transform }}
        />
      ) : (
        <span
          className="block rounded-full"
          style={{
            width: size - 3,
            height: size - 3,
            backgroundColor: PROVIDERS[provider].color,
          }}
        />
      )}
    </span>
  );
}

export function ProviderLogo({
  provider,
  size = 12,
  className = '',
  opticalAlign = 'center',
}: {
  provider: ProviderId;
  size?: number;
  className?: string;
  opticalAlign?: OpticalAlign;
}) {
  const meta = PROVIDERS[provider];
  const src = providerLogo(provider);
  if (src) {
    return (
      <Tooltip content={meta.label}>
        <img
          src={src}
          alt={meta.label}
          draggable={false}
          className={`shrink-0 ${className}`}
          style={{ width: size, height: size, transform: opticalTransform(provider, opticalAlign) }}
        />
      </Tooltip>
    );
  }
  // Fallback: brand-color dot for providers without a bundled logo yet.
  return (
    <Tooltip content={meta.label}>
      <span
        className={`shrink-0 rounded-full ${className}`}
        style={{ width: size, height: size, backgroundColor: meta.color }}
      />
    </Tooltip>
  );
}

export default ProviderLogo;
