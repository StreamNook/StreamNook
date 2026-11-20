// The accolade wall's measurements, in one place: the wall (AccoladeWall.tsx)
// draws with these numbers.

export const WALL = {
  /** Medallions across. */
  columns: 5,
  /** The panel's padding, per side. */
  pad: 14,
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
} as const;

/** Medallions across at a screen width: four below 340 css px, where five
 *  columns leave a label too little room for its longest single word. */
export function wallColumns(screenWidth: number): number {
  return screenWidth < 340 ? 4 : WALL.columns;
}
