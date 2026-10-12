// Where the notification surface (DynamicIsland) sits and how wide it is.
// Pure, so the narrow-window rule is tested without a title bar.

/** The surface's anchor in the title bar, measured by DynamicIsland. */
export interface IslandAnchor {
  left: number;
  top: number;
  width: number;
  height: number;
  /** Joined into the Home strip: the strip's width, which the surface fills. 0 standalone. */
  cover: number;
  radius: number;
  /** The stretch of the bar the surface may use, between the two icon clusters. */
  roomLeft: number;
  roomRight: number;
}

/** The open list is never narrower than this; less cannot show a notification. */
export const PANEL_MIN_WIDTH = 320;
/** Space kept between an open list and the window's edges. */
export const PANEL_EDGE_MARGIN = 8;
/** Space between the bar row and a list that opens below it. */
export const PANEL_DROP_GAP = 6;

export interface SurfaceGeometry {
  width: number;
  /** Horizontal move from where the surface would naturally sit. */
  shift: number;
  /** How far below the bar row the surface sits (0: in the row). */
  drop: number;
  /** The most height the surface may take, or null for no limit. */
  maxHeight: number | null;
}

/**
 * The surface's width and placement. In the bar row it stays between the icon
 * clusters (or fills the Home strip). Open as a list where that room is
 * narrower than PANEL_MIN_WIDTH (a narrow window, Compact View), it opens
 * below the bar row instead, at the list's own width, free to run under the
 * clusters' columns but kept inside the window.
 */
export function islandSurfaceGeometry(opts: {
  anchor: IslandAnchor | null;
  /** The width the surface wants now (the door, a preview, the strip, the list). */
  targetWidth: number;
  expanded: boolean;
  /** The list's own width for this window size. */
  panelWidth: number;
  viewportWidth: number;
  viewportHeight: number;
}): SurfaceGeometry {
  const { anchor, targetWidth, expanded, panelWidth, viewportWidth, viewportHeight } = opts;
  if (!anchor) return { width: targetWidth, shift: 0, drop: 0, maxHeight: null };
  const room = anchor.cover || Math.max(anchor.width, anchor.roomRight - anchor.roomLeft);
  if (expanded && room < PANEL_MIN_WIDTH) {
    const fit = Math.max(0, viewportWidth - 2 * PANEL_EDGE_MARGIN);
    const width = Math.min(Math.max(panelWidth, PANEL_MIN_WIDTH), fit);
    // Covering the strip it grows from the strip's left edge; standalone it
    // is centred on the door.
    const naturalLeft = anchor.cover ? anchor.left : anchor.left + anchor.width / 2 - width / 2;
    const left = Math.max(PANEL_EDGE_MARGIN, Math.min(naturalLeft, viewportWidth - PANEL_EDGE_MARGIN - width));
    const drop = anchor.height + PANEL_DROP_GAP;
    return {
      width,
      shift: left - naturalLeft,
      drop,
      maxHeight: Math.max(0, viewportHeight - anchor.top - drop - PANEL_EDGE_MARGIN),
    };
  }
  if (anchor.cover) return { width: targetWidth, shift: 0, drop: 0, maxHeight: null };
  // Standalone it grows from the door's centre, capped at the room between
  // the clusters, and slides back inside when the door sits off-centre there.
  const width = Math.min(targetWidth, room);
  const centredLeft = anchor.left + anchor.width / 2 - width / 2;
  const left = Math.max(anchor.roomLeft, Math.min(centredLeft, anchor.roomRight - width));
  return { width, shift: left - centredLeft, drop: 0, maxHeight: null };
}
