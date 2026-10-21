// Geometry of the phone's mini player when it is a transform of the full-size
// watch layer (WatchScreen): `p` 0 is the full player, 1 is the mini box. The
// layer is never laid out again while it shrinks or grows; it is moved and
// scaled about its top-left corner, the way Xtra minimizes its player.

/**
 * The layer's transform at `p`. At 1 the band's top-left corner sits on the
 * box's corner (`boxX`, `boxY`) and the band is `scale` times its full width.
 * `bandTop` is where the band starts inside the layer (the status-bar inset).
 */
export function miniLayerTransform(p: number, scale: number, boxX: number, boxY: number, bandTop: number): string {
  if (p <= 0) return 'none';
  return `translate(${p * boxX}px, ${p * (boxY - bandTop * scale)}px) scale(${1 - p * (1 - scale)})`;
}
