// Run with: npm test
//
// The notification list is squeezed into the gap between the title bar's icon
// clusters. At 956x704 that gap was about 105 px and in Compact View 81 px,
// too narrow to show a notification; there the list opens below the bar row.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { PANEL_DROP_GAP, PANEL_EDGE_MARGIN, PANEL_MIN_WIDTH, islandSurfaceGeometry, type IslandAnchor } from './islandGeometry';

// A door 72 px wide in a 105 px gap, as the bar measured at 956x704.
const narrow: IslandAnchor = { left: 470, top: 6, width: 72, height: 30, cover: 0, radius: 0, roomLeft: 453, roomRight: 558 };
// The same door with a wide gap, as at 1600 px.
const wide: IslandAnchor = { ...narrow, left: 760, roomLeft: 520, roomRight: 1080 };

const open = (anchor: IslandAnchor | null, viewportWidth = 956, viewportHeight = 704) =>
  islandSurfaceGeometry({ anchor, targetWidth: 360, expanded: true, panelWidth: 360, viewportWidth, viewportHeight });

test('a narrow gap opens the list below the bar at a usable width, inside the window', () => {
  const g = open(narrow);
  assert.ok(g.width >= PANEL_MIN_WIDTH);
  assert.equal(g.width, 360);
  assert.equal(g.drop, narrow.height + PANEL_DROP_GAP);
  // Centred on the door, where the window allows.
  const left = narrow.left + narrow.width / 2 - g.width / 2 + g.shift;
  assert.equal(left, narrow.left + narrow.width / 2 - 180);
  assert.ok(left >= PANEL_EDGE_MARGIN && left + g.width <= 956 - PANEL_EDGE_MARGIN);
  // It never runs off the bottom of the window.
  assert.equal(g.maxHeight, 704 - narrow.top - g.drop - PANEL_EDGE_MARGIN);
});

test('near a window edge the dropped list slides back inside', () => {
  const nearRight: IslandAnchor = { ...narrow, left: 900, roomLeft: 880, roomRight: 990 };
  const g = open(nearRight, 1000);
  const left = nearRight.left + nearRight.width / 2 - g.width / 2 + g.shift;
  assert.equal(left + g.width, 1000 - PANEL_EDGE_MARGIN);
  // A window narrower than the list: as wide as the window allows.
  const tiny = open({ ...narrow, left: 120, roomLeft: 100, roomRight: 190 }, 300);
  assert.equal(tiny.width, 300 - 2 * PANEL_EDGE_MARGIN);
  assert.equal(120 + 72 / 2 - tiny.width / 2 + tiny.shift, PANEL_EDGE_MARGIN);
});

test('a wide gap keeps the list in the bar row, between the clusters', () => {
  const g = open(wide, 1600, 900);
  assert.equal(g.drop, 0);
  assert.equal(g.maxHeight, null);
  assert.equal(g.width, 360);
  const left = wide.left + wide.width / 2 - g.width / 2 + g.shift;
  assert.ok(left >= wide.roomLeft && left + g.width <= wide.roomRight);
});

test('the closed door and a preview never drop, even in a narrow gap', () => {
  const door = islandSurfaceGeometry({ anchor: narrow, targetWidth: 72, expanded: false, panelWidth: 360, viewportWidth: 956, viewportHeight: 704 });
  assert.equal(door.drop, 0);
  assert.equal(door.width, 72);
  const preview = islandSurfaceGeometry({ anchor: narrow, targetWidth: 240, expanded: false, panelWidth: 360, viewportWidth: 956, viewportHeight: 704 });
  assert.equal(preview.drop, 0);
  assert.equal(preview.width, narrow.roomRight - narrow.roomLeft);
});

test('the Home strip is filled when it is wide enough, dropped below when not', () => {
  const strip: IslandAnchor = { left: 400, top: 6, width: 40, height: 32, cover: 520, radius: 16, roomLeft: 300, roomRight: 1000 };
  const filled = islandSurfaceGeometry({ anchor: strip, targetWidth: 520, expanded: true, panelWidth: 380, viewportWidth: 1280, viewportHeight: 720 });
  assert.deepEqual(filled, { width: 520, shift: 0, drop: 0, maxHeight: null });
  const thin = islandSurfaceGeometry({ anchor: { ...strip, cover: 200 }, targetWidth: 200, expanded: true, panelWidth: 380, viewportWidth: 1280, viewportHeight: 720 });
  assert.equal(thin.width, 380);
  assert.equal(thin.drop, strip.height + PANEL_DROP_GAP);
  assert.equal(thin.shift, 0);
});

test('no title bar (phone): unchanged', () => {
  assert.deepEqual(open(null), { width: 360, shift: 0, drop: 0, maxHeight: null });
});
