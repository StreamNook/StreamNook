import type { VodStoryboard, VodStoryboardVariant } from '../types';

/** One sprite frame in the shape Plyr's previewThumbnails reads. */
export interface PlyrSpriteFrame {
  startTime: number;
  endTime: number;
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PlyrThumbnailSet {
  frames: PlyrSpriteFrame[];
  height: number;
  width: number;
  urlPrefix: string;
}

/**
 * ONE variant, the largest frame. Plyr shows set 0 for every frame and
 * upgrades to the next set after 300 ms, so shipping the low sheet too would
 * decode both (the 800x3600 low sheet alone is ~11 MB RGBA) and load twice
 * per hover. The largest sheet fits the thumb at 1x.
 */
export function pickStoryboardVariant(sb: VodStoryboard): VodStoryboardVariant | null {
  let best: VodStoryboardVariant | null = null;
  for (const v of sb.variants) {
    const usable = v.count > 0 && v.rows > 0 && v.cols > 0 && v.interval_secs > 0 && v.images.length > 0;
    if (usable && (!best || v.height > best.height)) best = v;
  }
  return best;
}

/**
 * Expand one variant into Plyr's thumbnail set. Frame i sits in sheet
 * floor(i / (rows*cols)), at column i % cols and row floor((i % (rows*cols))
 * / cols), and covers [i, i+1) * interval seconds; the last frame stretches
 * to the duration so the tail always has a picture. A sheet the manifest
 * does not list ends the expansion (the frames before it stay valid).
 */
export function storyboardToPlyrThumbnails(
  v: VodStoryboardVariant,
  baseUrl: string,
  durationSecs: number,
): PlyrThumbnailSet[] {
  const perSheet = v.rows * v.cols;
  if (perSheet <= 0 || v.interval_secs <= 0 || v.count <= 0) return [];
  const frames: PlyrSpriteFrame[] = [];
  for (let i = 0; i < v.count; i++) {
    const image = v.images[Math.floor(i / perSheet)];
    if (!image) break;
    const idx = i % perSheet;
    const naturalEnd = (i + 1) * v.interval_secs;
    frames.push({
      startTime: i * v.interval_secs,
      endTime: i === v.count - 1 ? Math.max(naturalEnd, durationSecs) : naturalEnd,
      text: image,
      x: (idx % v.cols) * v.width,
      y: Math.floor(idx / v.cols) * v.height,
      w: v.width,
      h: v.height,
    });
  }
  return frames.length > 0 ? [{ frames, height: v.height, width: v.width, urlPrefix: baseUrl }] : [];
}
