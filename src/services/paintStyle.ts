// The 7TV paint → CSS engine, extracted from seventvService so it can run ANYWHERE
// (the desktop app AND the hosted overlay page). Pure functions, zero Tauri/store
// imports — this is the single source of truth for how a paint renders, so the
// in-app chat, the overlay builder, and the hosted overlay page can never drift.
//
// (seventvService re-exports computePaintStyle + PaintShadowMode from here for
// backward compatibility; existing imports are unchanged.)

import type { CSSProperties } from 'react';

// ─── v4 GraphQL paint types ─────────────────────────────────────────────────
export interface PaintLayer {
  id: string;
  ty: {
    __typename: string;
    // For LinearGradient
    angle?: number;
    repeating?: boolean;
    // For RadialGradient
    shape?: string;
    // For both gradients
    stops?: Array<{
      at: number;
      color: {
        hex: string;
        r: number;
        g: number;
        b: number;
        a: number;
      };
    }>;
    // For SingleColor
    color?: {
      hex: string;
      r: number;
      g: number;
      b: number;
      a: number;
    };
    // For Image
    images?: Array<{
      url: string;
      mime: string;
      size: number;
      scale: number;
      width: number;
      height: number;
      frameCount: number;
      localUrl?: string;
    }>;
  };
  opacity: number;
}

export interface PaintShadow {
  offsetX: number;
  offsetY: number;
  blur: number;
  color: {
    hex: string;
    r: number;
    g: number;
    b: number;
    a: number;
  };
}

export interface PaintV4 {
  id: string;
  name: string;
  description?: string;
  data: {
    layers: PaintLayer[];
    shadows: PaintShadow[];
  };
  selected?: boolean;
}

// ─── Layer → CSS helpers ────────────────────────────────────────────────────
const computeLinearGradientLayer = (layer: PaintLayer['ty'], opacity: number) => {
  if (layer.__typename !== 'PaintLayerTypeLinearGradient' || !layer.stops || layer.stops.length === 0) {
    return undefined;
  }

  const prefix = layer.repeating ? 'repeating-' : '';
  const stops = layer.stops.map((stop) => `${stop.color.hex} ${stop.at * 100}%`).join(', ');
  const gradient = `${prefix}linear-gradient(${layer.angle || 0}deg, ${stops})`;

  return {
    opacity,
    image: gradient
  };
};

const computeRadialGradientLayer = (layer: PaintLayer['ty'], opacity: number) => {
  if (layer.__typename !== 'PaintLayerTypeRadialGradient' || !layer.stops || layer.stops.length === 0) {
    return undefined;
  }

  const prefix = layer.repeating ? 'repeating-' : '';
  const shape = layer.shape === 'CIRCLE' ? 'circle' : 'ellipse';
  const stops = layer.stops.map((stop) => `${stop.color.hex} ${stop.at * 100}%`).join(', ');
  const gradient = `${prefix}radial-gradient(${shape}, ${stops})`;

  return {
    opacity,
    image: gradient
  };
};

/**
 * The image of a paint image layer that gets drawn: scale 1, and the animated
 * file whenever the layer animates. 7TV lists a layer's still `_static` variants
 * first, so the first scale-1 image is the still. Anything that caches a layer's
 * image must use this pick too: the cache is keyed by layer, and every surface
 * draws whichever file is stored there.
 */
export function pickPaintLayerImage<
  T extends { url: string; localUrl?: string; scale: number; frameCount: number },
>(
  images: readonly T[],
): T | undefined {
  const isAnimated = images.some((img) => img.frameCount > 1);
  return images.find((img) => img.scale === 1 && (!isAnimated || img.frameCount > 1));
}

const computeImageLayer = (layer: PaintLayer['ty'], opacity: number) => {
  if (layer.__typename !== 'PaintLayerTypeImage' || !layer.images) {
    return undefined;
  }

  const img = pickPaintLayerImage(layer.images);

  if (!img) {
    return undefined;
  }

  // Use local URL if available (desktop cache); the hosted overlay falls to the
  // CDN url, which is exactly what a browser source wants.
  const url = img.localUrl || img.url;

  return {
    opacity,
    image: `url("${url}")`
  };
};

const computeSingleColorLayer = (layer: PaintLayer['ty'], opacity: number) => {
  if (layer.__typename !== 'PaintLayerTypeSingleColor' || !layer.color) {
    return undefined;
  }

  return {
    opacity,
    color: layer.color.hex
  };
};

// Paint shadow render mode. 'all' = render every shadow the artist defined
// (default). 'one' = first shadow only (lighter visual). 'none' = no shadows
// (cleanest, most readable on busy backgrounds).
export type PaintShadowMode = 'all' | 'one' | 'none';

const computeDropShadows = (shadows: PaintShadow[], mode: PaintShadowMode = 'all') => {
  if (shadows.length === 0 || mode === 'none') {
    return undefined;
  }

  const picked = mode === 'one' ? shadows.slice(0, 1) : shadows;
  return picked
    .map((s) => `drop-shadow(${s.color.hex} ${s.offsetX}px ${s.offsetY}px ${s.blur}px)`)
    .join(' ');
};

// Bounded LRU memo for computePaintStyle. A chat with 100 visible messages from
// 30 distinct chatters used to compute 100 paint styles per render pass; with
// this cache it's at most one compute per unique (paint, color, shadowMode)
// combo. Returning the same object reference across calls also lets React's
// shallow prop comparison short-circuit downstream renders.
const PAINT_STYLE_CACHE_MAX = 256;
const paintStyleCache = new Map<string, CSSProperties>();

export const computePaintStyleUncached = (
  paint: PaintV4,
  userColor?: string,
  shadowMode: PaintShadowMode = 'all',
): CSSProperties => {
  const layers = paint.data.layers
    .map((layer) => {
      switch (layer.ty.__typename) {
        case 'PaintLayerTypeLinearGradient':
          return computeLinearGradientLayer(layer.ty, layer.opacity);
        case 'PaintLayerTypeRadialGradient':
          return computeRadialGradientLayer(layer.ty, layer.opacity);
        case 'PaintLayerTypeImage':
          return computeImageLayer(layer.ty, layer.opacity);
        case 'PaintLayerTypeSingleColor':
          return computeSingleColorLayer(layer.ty, layer.opacity);
        default:
          return undefined;
      }
    })
    .filter((l) => l !== undefined) as Array<{ opacity: number; image?: string; color?: string }>;

  const backgroundImages = layers.flatMap((l) => (l.image ? [l.image] : []));
  const backgroundColors = layers.flatMap((l) => (l.color ? [l.color] : []));

  // Use longhand properties to avoid React warning about mixing shorthand/longhand
  const backgroundImage = backgroundImages.length > 0 ? backgroundImages.join(', ') : undefined;
  const backgroundColor = backgroundColors.length > 0 ? backgroundColors[0] : (userColor || 'var(--user-color)');

  const filter = computeDropShadows(paint.data.shadows, shadowMode);

  const opacities = layers.map((l) => l.opacity).filter((o) => o < 1);
  const minOpacity = opacities.length > 0 ? Math.min(...opacities) : 1;

  const style: CSSProperties = {
    backgroundColor: backgroundColor,
    WebkitBackgroundClip: 'text',
    backgroundClip: 'text',
    backgroundSize: '100% 100%', // Per 7TV docs: ensures paint spans full text width
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
    color: 'transparent',
  };

  // Only add backgroundImage if we have gradient/image layers
  if (backgroundImage) {
    style.backgroundImage = backgroundImage;
  }

  if (filter) {
    style.filter = filter;
  }

  if (minOpacity < 1) {
    style.opacity = minOpacity;
  }

  return style;
};

// Compute the full CSS style for a paint.
// Memoized: same (paintId, userColor, shadowMode) returns the same object ref.
export const computePaintStyle = (
  paint: PaintV4,
  userColor?: string,
  shadowMode: PaintShadowMode = 'all',
): CSSProperties => {
  // Defensive: anonymous/test paints without an id fall through uncached.
  if (!paint?.id) {
    return computePaintStyleUncached(paint, userColor, shadowMode);
  }
  const key = `${paint.id}|${userColor ?? ''}|${shadowMode}`;
  const cached = paintStyleCache.get(key);
  if (cached) {
    // Touch-on-hit: re-insert to move to most-recent in Map iteration order.
    paintStyleCache.delete(key);
    paintStyleCache.set(key, cached);
    return cached;
  }
  const style = computePaintStyleUncached(paint, userColor, shadowMode);
  paintStyleCache.set(key, style);
  if (paintStyleCache.size > PAINT_STYLE_CACHE_MAX) {
    // Evict oldest (first key per insertion order).
    const oldest = paintStyleCache.keys().next().value;
    if (oldest !== undefined) paintStyleCache.delete(oldest);
  }
  return style;
};
