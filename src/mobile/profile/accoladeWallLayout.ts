// The accolade wall's measurements, in one place. The wall (AccoladeWall.tsx)
// draws with these numbers and the You tab's reserved slot is computed from the
// same ones, so the invisible space held while the wall is on its way is exactly as
// tall as a collapsed wall with two rows of medallions (the space is invisible).

export const WALL = {
  /** Medallions across. */
  columns: 5,
  /** The panel's padding, per side. */
  pad: 14,
  /** The panel's border, per side. */
  border: 1,
  /** The heading row, its gap to the grid included. */
  header: 28,
  /** Between medallion rows. */
  rowGap: 12,
  /** One medallion: 44 for the disc, 4 for the gap, 28 for a two-line label. */
  cell: 76,
  /** Above the "Show all" row and above the progress bar. */
  sectionGap: 12,
  /** The "Show all" row. */
  toggle: 44,
  /** The progress bar. */
  progress: 6,
  /** Rows a collapsed wall is held at before its real size is known. */
  collapsedRows: 2,
} as const;

/** How many rows a count of medallions takes. */
export function wallRows(count: number): number {
  return Math.ceil(Math.max(0, count) / WALL.columns);
}

/** The whole panel's height for a number of medallion rows, with or without the "Show all" row. */
export function wallHeight(rows: number, hasToggle: boolean): number {
  const grid = rows > 0 ? rows * WALL.cell + (rows - 1) * WALL.rowGap : 0;
  return (
    2 * WALL.border +
    2 * WALL.pad +
    WALL.header +
    grid +
    (hasToggle ? WALL.sectionGap + WALL.toggle : 0) +
    WALL.sectionGap +
    WALL.progress
  );
}

/** The height of the invisible space a held slot keeps: a collapsed wall with two rows and "Show all". */
export function collapsedWallHeight(): number {
  return wallHeight(WALL.collapsedRows, true);
}
