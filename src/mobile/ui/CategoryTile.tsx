// A category in a grid: its box art filling the column, then a chin below it
// with the name and how many watch it now. No card around it: the art is the
// object, and a glass frame only shrank it. Names never sit on the art.
import React, { useCallback, useState } from 'react';
import { gameBoxArt } from '../../utils/boxArt';
import { CHIN_CLASS, TILE_ART_CLASS, artPixelWidth } from './categoryTileLayout';

// One observer for every tile on the page: a grid mounts dozens at once, and
// one observer per tile would be dozens of observers doing a single job.
const widthWatchers = new WeakMap<Element, (width: number) => void>();
let sharedObserver: ResizeObserver | null = null;
function observeWidth(el: Element, onWidth: (width: number) => void): () => void {
  if (typeof ResizeObserver === 'undefined') {
    onWidth(el.getBoundingClientRect().width);
    return () => {};
  }
  sharedObserver ??= new ResizeObserver((entries) => {
    for (const e of entries) widthWatchers.get(e.target)?.(e.contentRect.width);
  });
  widthWatchers.set(el, onWidth);
  sharedObserver.observe(el);
  return () => {
    widthWatchers.delete(el);
    sharedObserver?.unobserve(el);
  };
}

export const CategoryTile: React.FC<{
  name: string;
  /** A finished picture URL (Kick's thumbnails come ready-made). */
  art?: string | null;
  /** Twitch box art, as a `{width}x{height}` template or a baked-size URL.
   *  The tile asks for it at the size it is drawn, in device pixels, rather
   *  than one fixed size that is too soft on a dense screen and too heavy on a
   *  narrow column. */
  boxArt?: string | null;
  /** Live viewers across the category, when the source counted them. Dims the
   *  line at zero. */
  viewers?: number | null;
  /** The words under the name, as Rust serves them with the category
   *  (`viewers_line`, one compact format for every shell); none when nobody
   *  counted. */
  viewersText?: string | null;
  /** Search results give the name two lines; browsing keeps one. */
  twoLineName?: boolean;
  onPress: () => void;
}> = ({ name, art, boxArt, viewers, viewersText, twoLineName = false, onPress }) => {
  const line = viewersText ?? null;
  // Rounded up to 30 px steps (in artPixelWidth), so a fold, a rotation or a
  // fractional column width lands on a handful of sizes the cache already
  // holds instead of a new request per pixel.
  const [pixelWidth, setPixelWidth] = useState<number | null>(null);
  const measure = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    return observeWidth(el, (w) => {
      if (w > 0) setPixelWidth(artPixelWidth(w, window.devicePixelRatio || 1));
    });
  }, []);
  // No URL until the tile knows its size, so it never fetches a guess first
  // and then the real one.
  const src = boxArt
    ? pixelWidth
      ? gameBoxArt(boxArt, pixelWidth, Math.round((pixelWidth * 4) / 3))
      : null
    : art;
  return (
    <button
      onClick={onPress}
      className="w-full text-left active:opacity-80 transition-opacity"
      aria-label={[name, line].filter(Boolean).join(', ')}
    >
      <div ref={measure} className={`${TILE_ART_CLASS} bg-surface`}>
        {src && (
          <img
            loading="lazy"
            decoding="async"
            src={src}
            alt=""
            className="absolute inset-0 w-full h-full object-cover"
            draggable={false}
          />
        )}
        {/* A hairline edge so art as dark as the page keeps its shape; on
            bright art it disappears. One device pixel, as the iPhone draws it:
            a full CSS pixel is four on this phone and reads as an outline. */}
        <span
          aria-hidden
          className="pointer-events-none absolute inset-0 rounded-[12px]"
          style={{
            boxShadow: 'inset 0 0 0 var(--sn-hairline, 1px) color-mix(in srgb, var(--color-text-primary) 10%, transparent)',
          }}
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
