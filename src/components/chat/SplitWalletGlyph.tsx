import type { ReactNode } from 'react';

// The wallet's mark for a channel with both channel points and Bits: one square
// split on the diagonal, the points art in the upper-left half and the Bits gem
// in the lower-right, both whole and the same size, with a short hairline
// across the gap between them. Nothing sits on a corner, so it never reads as an
// unread badge. The iPhone's SplitWalletGlyph (WalletButton.swift) draws the
// same geometry on the same 22px grid.

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
  gemUrl,
  size = 22,
  className = '',
}: {
  /** Draws the points art at the size it gets inside the glyph. */
  points: (size: number) => ReactNode;
  gemUrl: string;
  size?: number;
  className?: string;
}) {
  const u = size / 22;
  const mark = size / 2;
  return (
    <span className={`relative inline-block flex-shrink-0 ${className}`} style={{ width: size, height: size }}>
      <span className="absolute flex" style={{ left: 0.5 * u, top: 0.5 * u, width: mark, height: mark }}>
        {points(mark)}
      </span>
      <img
        src={gemUrl}
        alt=""
        draggable={false}
        className="absolute object-contain"
        style={{ left: 10.5 * u, top: 10.5 * u, width: mark, height: mark }}
      />
      {/* Only across the gap: 10px long, ending just inside the marks' width, and
          centered on the gap you see (a gem's nearest point sits farther from the
          middle than the round points art's), so it rides 1px toward the gem. */}
      <svg className="absolute inset-0" width={size} height={size} viewBox="0 0 22 22" aria-hidden="true">
        <line
          x1={15.2}
          y1={8.13}
          x2={8.13}
          y2={15.2}
          stroke="var(--color-text-secondary)"
          strokeOpacity={0.45}
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    </span>
  );
}
