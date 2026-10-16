// A category in a grid: its box art filling the column, then a chin below it
// with the name and how many watch it now. No card around it: the art is the
// object, and a glass frame only shrank it. Names never sit on the art.
import React from 'react';
import { CHIN_CLASS, TILE_ART_CLASS, viewersLine } from './categoryTileLayout';

export const CategoryTile: React.FC<{
  name: string;
  art: string | null | undefined;
  /** Live viewers across the category, when the source counted them. */
  viewers?: number | null;
  /** Search results give the name two lines; browsing keeps one. */
  twoLineName?: boolean;
  onPress: () => void;
}> = ({ name, art, viewers, twoLineName = false, onPress }) => {
  const line = viewersLine(viewers);
  return (
    <button
      onClick={onPress}
      className="w-full text-left active:opacity-80 transition-opacity"
      aria-label={[name, line].filter(Boolean).join(', ')}
    >
      <div className={`${TILE_ART_CLASS} bg-surface`}>
        {art && (
          <img
            loading="lazy"
            decoding="async"
            src={art}
            alt=""
            className="absolute inset-0 w-full h-full object-cover"
            draggable={false}
          />
        )}
        {/* A hairline edge so art as dark as the page keeps its shape; on
            bright art it disappears. */}
        <span
          aria-hidden
          className="pointer-events-none absolute inset-0 rounded-[12px]"
          style={{ boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--color-text-primary) 10%, transparent)' }}
        />
      </div>
      <div className={CHIN_CLASS}>
        <div
          className={`text-[12.5px] font-medium leading-[17px] text-textPrimary ${
            twoLineName ? 'line-clamp-2' : 'truncate'
          }`}
        >
          {name}
        </div>
        {line && (
          // Nobody live is the least useful fact, so it is a step quieter.
          <div
            className={`text-[12px] leading-4 text-textSecondary truncate ${viewers === 0 ? 'opacity-70' : ''}`}
          >
            {line}
          </div>
        )}
      </div>
    </button>
  );
};
