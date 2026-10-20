import { useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';

// The wallet's mark for a channel with both channel points and Bits: one square
// split on the diagonal, the points art in the upper-left half and the Bits gem
// in the lower-right, both whole and the same size, with a hairline across the
// gap between them. Nothing sits on a corner, so it never reads as an unread
// badge. Where each part goes is Rust's (services/wallet_glyph.rs): it measures
// the channel's art, so round, full-bleed and padded art all sit the same gap
// from the gem. The iPhone's SplitWalletGlyph (WalletButton.swift) draws the
// same layout.

/** Rust's `SplitLayout`, on its own grid. */
interface SplitLayout {
  grid: number;
  mark: number;
  art_at: number;
  gem_at: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  shift_x: number;
  shift_y: number;
}

/** Rust's `SplitLayout::FALLBACK`: Twitch's ring against the 100-bit gem,
 *  drawn until the measured layout arrives. */
const FALLBACK: SplitLayout = {
  grid: 24,
  mark: 11.5,
  art_at: 2.42,
  gem_at: 12.5,
  x1: 19.74,
  y1: 8.24,
  x2: 8.24,
  y2: 19.74,
  shift_x: -0.73,
  shift_y: -1.35,
};

// One answer per art pair for the whole page; Rust caches it too.
const layouts = new Map<string, SplitLayout>();

function useSplitLayout(pointsIcon: string | null, gem: string): SplitLayout {
  const key = `${pointsIcon ?? ''}|${gem}`;
  const [, setReady] = useState(0);
  useEffect(() => {
    if (layouts.has(key)) return;
    let live = true;
    invoke<SplitLayout>('wallet_split_layout', { pointsIcon, gem })
      .then((layout) => {
        layouts.set(key, layout);
        if (live) setReady((n) => n + 1);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [key, pointsIcon, gem]);
  return layouts.get(key) ?? FALLBACK;
}

/** A bonus chest waiting: the whole wallet button becomes the chest, in the
 *  theme's warning amber with a soft glow, never a mark on a corner. The
 *  wiggle is `chest-attention` (transform only, off under reduced motion). */
export function WalletChestGlyph({ size = 18 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      className="text-warning drop-shadow-[0_0_3px_color-mix(in_srgb,var(--color-warning)_40%,transparent)]"
    >
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M2 22v-9l2.417-4.029A2 2 0 0 1 6.132 8h11.736a2 2 0 0 1 1.715.971L22 13v9H2Zm18-2v-5h-7v2h-2v-2H4v5h16Zm-2.132-10 1.8 3H4.332l1.8-3h11.736Z"
      />
    </svg>
  );
}

export function SplitWalletGlyph({
  points,
  pointsIcon,
  gemUrl,
  size = 24,
  className = '',
}: {
  /** Draws the points art at the size it gets inside the glyph. */
  points: (size: number) => ReactNode;
  /** The channel's points art URL (null: Twitch's glyph), which Rust measures. */
  pointsIcon: string | null;
  gemUrl: string;
  size?: number;
  className?: string;
}) {
  const l = useSplitLayout(pointsIcon, gemUrl);
  const u = size / l.grid;
  const mark = l.mark * u;
  return (
    <span className={`relative inline-block flex-shrink-0 ${className}`} style={{ width: size, height: size }}>
      <span className="absolute inset-0" style={{ transform: `translate(${l.shift_x * u}px, ${l.shift_y * u}px)` }}>
        <span className="absolute flex" style={{ left: l.art_at * u, top: l.art_at * u, width: mark, height: mark }}>
          {points(mark)}
        </span>
        <img
          src={gemUrl}
          alt=""
          draggable={false}
          className="absolute object-contain"
          style={{ left: l.gem_at * u, top: l.gem_at * u, width: mark, height: mark }}
        />
        <svg className="absolute inset-0" width={size} height={size} viewBox={`0 0 ${l.grid} ${l.grid}`} aria-hidden="true">
          <line
            x1={l.x1}
            y1={l.y1}
            x2={l.x2}
            y2={l.y2}
            stroke="var(--color-text-secondary)"
            strokeOpacity={0.45}
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
          />
        </svg>
      </span>
    </span>
  );
}
