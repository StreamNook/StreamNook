// First-load placeholders shaped like what lands in their place, so a list
// does not jump when it arrives: a quiet pulse on the surface tokens, no
// shimmer gradients.
import React from 'react';
import { CHIN_CLASS, TILE_ART_CLASS, TILE_GRID_CLASS } from './categoryTileLayout';

/** MobileStreamCard's shape: the preview inside the card's inset, then the
 *  title, channel and category lines. */
export const SkeletonCards: React.FC<{ count?: number }> = ({ count = 3 }) => (
  <div className="flex flex-col gap-3 px-4 pb-4">
    {Array.from({ length: count }, (_, i) => (
      <div key={i} className="glass-panel media-card p-2 animate-pulse">
        <div className="sn-card-thumb w-full aspect-video bg-surface mb-2" />
        <div className="px-0.5 pb-0.5">
          <div className="h-4 w-3/4 rounded bg-surface mb-1.5" />
          <div className="h-3.5 w-1/3 rounded bg-surface mb-1.5" />
          <div className="h-3 w-1/4 rounded bg-surface" />
        </div>
      </div>
    ))}
  </div>
);

/** CategoryTile's shape: the 3:4 art, then the chin's two lines. */
export const SkeletonTiles: React.FC<{ count?: number; className?: string }> = ({ count = 9, className = '' }) => (
  <div className={`${TILE_GRID_CLASS} px-4 ${className}`}>
    {Array.from({ length: count }, (_, i) => (
      <div key={i} className="animate-pulse">
        <div className={`${TILE_ART_CLASS} bg-surface`} />
        <div className={CHIN_CLASS}>
          <div className="h-3 w-4/5 rounded bg-surface mt-0.5" />
          <div className="h-2.5 w-1/2 rounded bg-surface mt-1.5" />
        </div>
      </div>
    ))}
  </div>
);
