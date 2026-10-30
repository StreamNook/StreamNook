// A QR code for a song's link, drawn in the page (SongQrDialog.tsx). uqr
// encodes the link at error correction level H, which recovers 30% of the
// code; this turns its modules into one SVG path of rounded squares, draws
// the three finder patterns as rounded rings around rounded dots, and clears
// a square at the center for the StreamNook logo. The square covers at most
// 22% of the code's side (about 5% of its area) and never touches a finder
// pattern, the timing rows or the format areas, so the code scans with room
// to spare. Pure, so it is tested without a page.
import { encode, QrCodeDataType } from 'uqr';

/** A module's corner radius, as a share of the module. */
const MODULE_RADIUS = 0.35;
/** A finder pattern's corner radius, as a share of its side. */
const FINDER_RADIUS = 0.3;
/** The most of the code's side the logo's square may cover. */
export const KNOCKOUT_SHARE = 0.22;
/** The light margin around the code, in modules. */
export const QUIET_ZONE = 4;

/** A square of modules. */
export interface QrBox {
  x: number;
  y: number;
  size: number;
}

export interface QrModel {
  /** Modules on a side, without the quiet zone. */
  size: number;
  /** The square cleared for the logo (its white backing), in modules. */
  knockout: QrBox;
  /** The logo inside it: one module narrower, centered. */
  logo: QrBox;
  /** Every dark module outside the finders and the knockout, as one rounded square each. */
  modules: string;
  /** The three finder patterns: each ring and its dot (fill-rule evenodd). */
  finders: string;
  /** The dark modules, row by row. */
  dark: boolean[][];
  /** Each module's role, row by row (uqr's QrCodeDataType). */
  types: QrCodeDataType[][];
}

/** A coordinate as the path writes it: three decimals at most. */
const n = (v: number) => String(Math.round(v * 1000) / 1000);

/** A rounded square at (x, y), `w` on a side with corner radius `r`, drawn clockwise. */
function roundSquare(x: number, y: number, w: number, r: number): string {
  const s = n(w - 2 * r);
  const ns = n(-(w - 2 * r));
  const a = `a${n(r)} ${n(r)} 0 0 1`;
  return `M${n(x + r)} ${n(y)}h${s}${a} ${n(r)} ${n(r)}v${s}${a} ${n(-r)} ${n(r)}h${ns}${a} ${n(-r)} ${n(-r)}v${ns}${a} ${n(r)} ${n(-r)}z`;
}

/** The roles the logo's square keeps clear of: the finders, the timing rows,
 *  and the format and version areas (uqr marks those Function). The
 *  alignment patterns are data's helpers and level H covers them. */
const KEPT: ReadonlySet<QrCodeDataType> = new Set([
  QrCodeDataType.Position,
  QrCodeDataType.Timing,
  QrCodeDataType.Function,
]);

/** Whether any kept module lies in `box` or right beside it. */
function nearKept(types: QrCodeDataType[][], box: QrBox): boolean {
  const from = Math.max(0, box.x - 1);
  const to = Math.min(types.length - 1, box.x + box.size);
  for (let y = from; y <= to; y++) {
    for (let x = from; x <= to; x++) {
      if (KEPT.has(types[y][x])) return true;
    }
  }
  return false;
}

/** The centered square for the logo: odd on a side (a code's side is odd, so
 *  it centers on the middle module), as large as 22% of the side allows, and
 *  smaller while it would touch a kept module. */
function knockoutFor(size: number, types: QrCodeDataType[][]): QrBox {
  let side = Math.floor(size * KNOCKOUT_SHARE);
  if (side % 2 === 0) side -= 1;
  for (; side >= 3; side -= 2) {
    const at = (size - side) / 2;
    const box = { x: at, y: at, size: side };
    if (!nearKept(types, box)) return box;
  }
  return { x: (size - 1) / 2, y: (size - 1) / 2, size: 0 };
}

/** Whether (x, y) lies in `box`. */
export const inBox = (box: QrBox, x: number, y: number) =>
  x >= box.x && x < box.x + box.size && y >= box.y && y < box.y + box.size;

/** The code for `text`, ready to draw. */
export function qrModel(text: string): QrModel {
  const qr = encode(text, { ecc: 'H', border: 0 });
  const { size, data: dark, types } = qr;
  const knockout = knockoutFor(size, types);
  const logo = knockout.size
    ? { x: knockout.x + 0.5, y: knockout.y + 0.5, size: knockout.size - 1 }
    : { x: knockout.x, y: knockout.y, size: 0 };

  const parts: string[] = [];
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (!dark[y][x] || types[y][x] === QrCodeDataType.Position || inBox(knockout, x, y)) continue;
      parts.push(roundSquare(x, y, 1, MODULE_RADIUS));
    }
  }

  const finders = [
    [0, 0],
    [size - 7, 0],
    [0, size - 7],
  ]
    .map(
      ([x, y]) =>
        roundSquare(x, y, 7, 7 * FINDER_RADIUS) +
        roundSquare(x + 1, y + 1, 5, 5 * FINDER_RADIUS) +
        roundSquare(x + 2, y + 2, 3, 3 * FINDER_RADIUS),
    )
    .join('');

  return { size, knockout, logo, modules: parts.join(''), finders, dark, types };
}

/** A link to show as a code. */
export interface SongQrRequest {
  url: string;
  /** Its service, as Rust names links ("Spotify"). */
  service: string;
  /** A search page rather than the track's own link. */
  search: boolean;
}

/** The dialog's heading for a link. */
export const qrHeading = (r: Pick<SongQrRequest, 'service' | 'search'>) =>
  r.search ? `Search ${r.service} on your phone` : `Open in ${r.service} on your phone`;

/** A link as the dialog shows it: its host (without `www.`) and path, no
 *  scheme, query or trailing slash. The whole link when it does not read. */
export function shortLink(url: string): string {
  try {
    const u = new URL(url);
    const host = u.host.replace(/^www\./, '');
    const path = decodeURIComponent(u.pathname).replace(/\/+$/, '');
    return `${host}${path}`;
  } catch {
    return url;
  }
}
