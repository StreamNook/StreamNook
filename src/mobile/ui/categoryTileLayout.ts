// The category tile's shared geometry, for the tile and its skeleton. Its
// viewers line comes from Rust with each category (`viewers_line`).

/** The grid both browse screens lay tiles in. */
export const TILE_GRID_CLASS = 'grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 gap-x-2.5 gap-y-3.5';
/** The art's frame: 3:4, 12 px corners. */
export const TILE_ART_CLASS = 'relative w-full aspect-[3/4] overflow-hidden rounded-[12px]';
/** The chin keeps the height of a name line and a viewers line, so a tile
 *  without a count still lines up with its row. */
export const CHIN_CLASS = 'mt-[5px] min-h-[33px]';

/** The box art width to ask for: the drawn width in device pixels, rounded up
 *  to a 30 px step. The height follows at 3:4. */
export function artPixelWidth(drawnCssWidth: number, devicePixelRatio: number): number {
  return Math.ceil((Math.max(drawnCssWidth, 60) * devicePixelRatio) / 30) * 30;
}
