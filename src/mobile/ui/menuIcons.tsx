// The glyphs the phone's menus share for favorites and subscribing. Favorite
// is desktop's glass heart, as on the Favorites shelf and desktop's stream
// menu; Subscribe always wears the channel's badge art from Rust and keeps an
// empty slot while it loads, so the row never shows a stand-in and never
// shifts when the art lands.
import React from 'react';
import { Heart } from 'lucide-react';

export function favoriteLabel(on: boolean): string {
  return on ? 'Unfavorite' : 'Favorite';
}

/** Desktop's glass heart (pink glass, a white edge fading down, a soft pink
 *  glow) when on, a quiet outline when off. It carries its own gradients since
 *  the phone shell never mounts desktop's. */
export const GlassHeart: React.FC<{ on: boolean; size: number }> = ({ on, size }) =>
  on ? (
    <>
      <svg width="0" height="0" className="absolute pointer-events-none" aria-hidden>
        <defs>
          <linearGradient id="sn-m-glass-heart-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="rgba(255, 255, 255, 0.4)" />
            <stop offset="30%" stopColor="rgba(236, 72, 153, 0.2)" />
            <stop offset="100%" stopColor="rgba(236, 72, 153, 0.6)" />
          </linearGradient>
          <linearGradient id="sn-m-glass-heart-stroke" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="rgba(255, 255, 255, 0.8)" />
            <stop offset="100%" stopColor="rgba(255, 255, 255, 0.1)" />
          </linearGradient>
        </defs>
      </svg>
      <Heart
        size={size}
        fill="url(#sn-m-glass-heart-fill)"
        stroke="url(#sn-m-glass-heart-stroke)"
        strokeWidth={1.5}
        className={`shrink-0 ${
          size < 16
            ? 'drop-shadow-[0_3px_6px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]'
            : 'drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]'
        }`}
        aria-hidden
      />
    </>
  ) : (
    <Heart size={size} strokeWidth={1.5} className="shrink-0 text-textSecondary" aria-hidden />
  );

export const SubscribeBadge: React.FC<{ url: string | null; size: number }> = ({ url, size }) =>
  url ? (
    <img src={url} alt="" draggable={false} style={{ width: size, height: size }} />
  ) : (
    <span aria-hidden className="block" style={{ width: size, height: size }} />
  );
