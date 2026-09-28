import { useId, type CSSProperties, type ReactNode } from 'react';
import { IS_WEBKITGTK } from '../utils/platform';

/**
 * The StreamNook mark: nine isometric cubes forming a Penrose impossible
 * triangle, with each cube marching one slot around the perimeter on a 1.5s
 * loop.
 *
 * This is a faithful port of `scripts/penrose-source.html`, the same source the
 * gold subscriber badge and the five tenure badges are baked from. Geometry,
 * document order and the cube 8 hand-off are copied from it verbatim. Only two
 * things differ, and neither touches the drawing:
 *
 *   1. Face colours come from the theme accent instead of being gold.
 *   2. The viewBox is padded, because the source's box leaves under two units of
 *      margin and reads as clipped once the cubes march.
 *
 * Do not "improve" the stacking. The order is hand-tuned, several pairs of cubes
 * are deliberately drawn in the wrong depth order, and that is exactly what makes
 * the triangle read as impossible. A consistent depth sort turns it into an
 * ordinary pile of blocks, and partial corrections make panels appear to lie
 * across cubes they should be behind.
 *
 * The one thing added on top is the seam strips. The three faces share exact
 * edges and each is antialiased separately, so those shared edges come out
 * slightly translucent and whatever sits behind shows through as a hairline.
 * Each internal seam therefore gets a narrow strip drawn directly beneath it,
 * painted with the SAME GRADIENT as the darker of its two faces, so the line
 * renders as a crisp continuation of that face instead of a third colour.
 * That is why the gradients are defined in cube-local user space rather than
 * per-shape bounding boxes: the strip must sample identical colours to the
 * face it impersonates.
 *
 * Approaches already tried and rejected, so they do not come back:
 * - Stroking the faces: a stroke grows outward and bleeds onto the next cube.
 * - A full-size opaque hexagon underneath: its own antialiased edge fattens
 *   the silhouette by a part pixel, a light rim that breathes as cubes move.
 * - An inset hexagon in one mid tone: the seam then reads LIGHT against the
 *   dark left face and DARK against the light top face, a half-and-half line.
 *   And for the split cubes it rode with the bottom-pinned face, so other
 *   cubes slid between it and the seam mid-loop and showed through the gap.
 *
 * Strip placement for the split cubes is deliberate, per seam: each strip
 * lives in the group where nothing that matters can interpose between it and
 * the two faces it backs. Worked out case by case; see the body comments.
 *
 * The rule this file has already had to learn several times: changes here are
 * judged by eye, never by counting changed pixels.
 */

const FACE_TOP = '0,0 17.32,10 0,20 -17.32,10';
const FACE_RIGHT = '17.32,10 17.32,30 0,40 0,20';
const FACE_LEFT = '-17.32,10 0,20 0,40 -17.32,30';

// One strip per internal seam, 0.75 units each side of the seam line, ending a
// unit short of the silhouette so nothing pokes past the outline. The vertical
// seam runs (0,20)-(0,40); the diagonals run (0,20)-(±17.32,10).
const SEAM_V = '-0.75,20 0.75,20 0.75,39 -0.75,39';
const SEAM_TR = '0.375,20.65 16.825,11.15 16.075,9.85 -0.375,19.35';
const SEAM_TL = '-0.375,20.65 -16.825,11.15 -16.075,9.85 0.375,19.35';

// Across the whole march the cubes occupy x [22.68, 135.32], y [20, 150]. This
// is that box centred with 8 units of margin on every side. The Linux layered
// stylesheet carries its 128.64 x 146 ratio and the march steps as fractions.
const VIEW_BOX = '14.68 12 128.64 146';
const VIEW_BOX_WIDTH = 128.64;
const VIEW_BOX_HEIGHT = 146;

interface PenroseMarchProps {
  /** Rendered width in CSS pixels. Height follows the source aspect ratio. */
  size?: number;
  className?: string;
}

const PenroseMarch = ({ size = 100, className }: PenroseMarchProps) => {
  // Two loaders can be mounted at once, so gradient ids have to be per
  // instance. useId emits colons, which are not safe inside url(#...).
  const uid = useId().replace(/:/g, '');
  const topId = `pm-top-${uid}`;
  const rightId = `pm-right-${uid}`;
  const leftId = `pm-left-${uid}`;
  const cubeId = `pm-cube-${uid}`;

  const top = <polygon points={FACE_TOP} fill={`url(#${topId})`} />;
  const right = <polygon points={FACE_RIGHT} fill={`url(#${rightId})`} />;
  const left = <polygon points={FACE_LEFT} fill={`url(#${leftId})`} />;

  // Each seam strip impersonates the darker of its two faces.
  const seamV = <polygon className="pm-seam" points={SEAM_V} fill={`url(#${leftId})`} />;
  const seamTR = <polygon className="pm-seam" points={SEAM_TR} fill={`url(#${rightId})`} />;
  const seamTL = <polygon className="pm-seam" points={SEAM_TL} fill={`url(#${leftId})`} />;

  // The moving groups, in the source's document order. Each body carries its
  // own placement; the march is applied around it, and since both are plain
  // translations the order they compose in does not matter.
  const layers: { flow: 'a' | 'b' | 'c'; phase?: 'a' | 'b'; hidden?: boolean; body: ReactNode }[] = [
    // Lowest layers: partial faces that later cubes are meant to cover.

    // Cubes 6, 7 and 8 are split across layers, so their seam strips are
    // placed seam by seam: each rides in a group where no cube that is
    // supposed to show can slide between the strip and its seam.

    // Cube 6's left face, covered by cube 7's top. The top|left strip lives
    // down here so cube 7's top still covers it whenever it covers the left
    // face; when cube 7 is elsewhere, it backs the seam.
    { flow: 'b', body: <g transform="translate(66, 95)">{seamTL}{left}</g> },

    // Cube 7's top face, above cube 6's left, below every cube 8 face. Both
    // diagonal strips sit here; the only things that ever interpose above
    // them are cube 8's faces, which are opaque and meant to cover.
    { flow: 'c', body: <g transform="translate(40, 110)">{seamTR}{seamTL}{top}</g> },

    // Cube 8's top face, covered by cube 9. Diagonal strips as above.
    { flow: 'c', body: <g transform="translate(40, 80)">{seamTR}{seamTL}{top}</g> },

    // Cube 8's right face, low copy. Opaque only in the second half of the
    // loop, where it needs to sit under the cubes drawn after it. The
    // vertical strip rides with EACH right-face copy, beneath it: strips
    // must always sit under both of their faces, or they paint a band onto
    // the lower one, and since only cube 8 would carry that band, the band
    // would jump to a different slot at the loop reset.
    { flow: 'c', phase: 'b', hidden: true, body: <g transform="translate(40, 80)">{seamV}{right}</g> },

    { flow: 'c', body: <use href={`#${cubeId}`} x="40" y="50" /> },

    { flow: 'a', body: <use href={`#${cubeId}`} x="40" y="20" /> },
    { flow: 'a', body: <use href={`#${cubeId}`} x="66" y="35" /> },
    { flow: 'a', body: <use href={`#${cubeId}`} x="92" y="50" /> },
    { flow: 'b', body: <use href={`#${cubeId}`} x="118" y="65" /> },
    { flow: 'b', body: <use href={`#${cubeId}`} x="92" y="80" /> },

    // Cube 6's remaining faces, with its vertical and top|right strips.
    // Verified: cube 7's top never reaches either strip's footprint.
    { flow: 'b', body: <g transform="translate(66, 95)">{seamV}{seamTR}{top}{right}</g> },

    // Cube 7's remaining faces. The vertical strip MUST live up here, not
    // with the pinned top face: cube 6's body slides between the low layer
    // and this one late in the loop, and showed through the seam.
    { flow: 'c', body: <g transform="translate(40, 110)">{seamV}{right}{left}</g> },

    // Cube 8's right face, high copy. Opaque only in the first half.
    { flow: 'c', phase: 'a', body: <g transform="translate(40, 80)">{seamV}{right}</g> },

    // Cube 8's left face. Its side of the vertical seam is backed by the
    // strip travelling with the active right-face copy below.
    { flow: 'c', body: <g transform="translate(40, 80)">{left}</g> },
  ];

  const defs = (
    <defs>
      {/* Cube-local user-space coordinates, equivalent to the source file's
          bounding-box gradients on the faces, and shared exactly by the seam
          strips. Face extents: top x -17.32..17.32 y 0..20, right and left
          y 10..40 with midlines at x ±8.66. On Linux the separate layers
          reach these by id; ids are document-wide, so one set serves all. */}
      <linearGradient id={topId} gradientUnits="userSpaceOnUse" x1="-17.32" y1="0" x2="17.32" y2="20">
        <stop offset="0%" className="pm-top-hi" />
        <stop offset="100%" className="pm-top-lo" />
      </linearGradient>
      <linearGradient id={rightId} gradientUnits="userSpaceOnUse" x1="8.66" y1="10" x2="8.66" y2="40">
        <stop offset="0%" className="pm-right-hi" />
        <stop offset="100%" className="pm-right-lo" />
      </linearGradient>
      <linearGradient id={leftId} gradientUnits="userSpaceOnUse" x1="-8.66" y1="10" x2="-8.66" y2="40">
        <stop offset="0%" className="pm-left-hi" />
        <stop offset="100%" className="pm-left-lo" />
      </linearGradient>
      {/* overflow visible or the symbol viewport clips every cube */}
      <symbol id={cubeId} overflow="visible">
        {seamV}
        {seamTR}
        {seamTL}
        {top}
        {right}
        {left}
      </symbol>
    </defs>
  );
  const flowClass = (layer: (typeof layers)[number]) =>
    `pm-cube pm-flow-${layer.flow}${layer.phase ? ` pm-phase-${layer.phase}` : ''}`;

  if (!IS_WEBKITGTK) {
    return (
      <svg
        className={className ? `penrose-march ${className}` : 'penrose-march'}
        viewBox={VIEW_BOX}
        width={size}
        height={Math.round((size * VIEW_BOX_HEIGHT) / VIEW_BOX_WIDTH)}
        role="img"
        aria-label="Loading"
        style={{ display: 'block' }}
      >
        {defs}
        {layers.map((layer, i) => (
          <g key={i} className={flowClass(layer)} style={layer.hidden ? { opacity: 0 } : undefined}>
            {layer.body}
          </g>
        ))}
      </svg>
    );
  }

  // Linux (WebKitGTK): every moving group is its own HTML layer with its own
  // copy of the viewBox, and the march is a CSS transform on that layer.
  // WebKitGTK drives transform animations on SVG groups from the main thread,
  // so the mark froze whenever the page was busy booting; on an HTML layer the
  // compositor runs it. The drawing inside each layer is unchanged.
  return (
    <div
      className={className ? `penrose-march penrose-march--layered ${className}` : 'penrose-march penrose-march--layered'}
      role="img"
      aria-label="Loading"
      // Width through a custom property rather than an inline width, so a
      // caller's class can still size the mark; the height follows from the
      // aspect ratio in the stylesheet.
      style={{ '--pm-size': `${size}px` } as CSSProperties}
    >
      <svg className="pm-defs" width="0" height="0" aria-hidden="true" focusable="false">
        {defs}
      </svg>
      {layers.map((layer, i) => (
        <div key={i} className={flowClass(layer)} style={layer.hidden ? { opacity: 0 } : undefined}>
          <svg viewBox={VIEW_BOX} aria-hidden="true" focusable="false">
            {layer.body}
          </svg>
        </div>
      ))}
    </div>
  );
};

export default PenroseMarch;
