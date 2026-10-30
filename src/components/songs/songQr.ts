// A QR code for a song's link, drawn in the page (SongQrDialog.tsx). uqr
// encodes the link at the highest error correction that holds it: level H
// (recovers 30% of the code) for most links, Q (25%) or M (15%) for long ones;
// a link too long for M gets no code at all (`qrModel` answers null and the
// QR buttons are not drawn). This turns the modules into one SVG path of
// rounded squares, draws the three finder patterns as rounded rings around
// rounded dots, and at H and Q clears a square at the center for the
// StreamNook logo: at most 22% of the side at H and 18% at Q (about 5% and 3%
// of the area, well inside each level's recovery), never touching a finder
// pattern, the timing rows or the format areas. Pure, so it is tested
// without a page.
import { encode, QrCodeDataType } from 'uqr';

/** A module's corner radius, as a share of the module. */
const MODULE_RADIUS = 0.35;
/** A finder pattern's corner radius, as a share of its side. */
const FINDER_RADIUS = 0.3;
/** The levels tried, strongest first. */
const LEVELS = ['H', 'Q', 'M'] as const;
export type QrLevel = (typeof LEVELS)[number];
/** The share of the code each level recovers. */
export const RECOVERY: Readonly<Record<QrLevel, number>> = { H: 0.3, Q: 0.25, M: 0.15 };
/** The most of the code's side the logo's square may cover at each level
 *  (none at M, whose recovery is too thin to spend on a logo). */
export const KNOCKOUT_SHARE: Readonly<Record<QrLevel, number>> = { H: 0.22, Q: 0.18, M: 0 };
/** The most a code at level M holds: the largest code's bytes (a link is
 *  always encoded as bytes: `https` is outside the shorter alphabets). */
export const QR_MAX_BYTES = 2331;
/** The light margin around the code, in modules. */
export const QUIET_ZONE = 4;

const utf8 = new TextEncoder();

/** Whether a code can hold `text` at all (level M at the largest size). */
export function qrFits(text: string): boolean {
  return text.length > 0 && utf8.encode(text).length <= QR_MAX_BYTES;
}

/** A square of modules. */
export interface QrBox {
  x: number;
  y: number;
  size: number;
}

export interface QrModel {
  /** The error correction level it is encoded at. */
  level: QrLevel;
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
 *  it centers on the middle module), as large as the level's share of the
 *  side allows, and smaller while it would touch a kept module. None at M. */
function knockoutFor(size: number, types: QrCodeDataType[][], level: QrLevel): QrBox {
  let side = Math.floor(size * KNOCKOUT_SHARE[level]);
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

/** `text` encoded at the strongest level that holds it, or null when none does. */
function encodeStrongest(text: string): { level: QrLevel; qr: ReturnType<typeof encode> } | null {
  if (!qrFits(text)) return null;
  for (const level of LEVELS) {
    try {
      return { level, qr: encode(text, { ecc: level, border: 0 }) };
    } catch {
      // Too long for this level ("Data too long"): try a lighter one.
    }
  }
  return null;
}

// The last codes made, so the dialog and its buttons ask once per link.
const made = new Map<string, QrModel | null>();
const MADE_KEPT = 16;

/** The code for `text`, ready to draw; null when the link is too long for any
 *  code (the QR buttons are then not drawn). Never throws. */
export function qrModel(text: string): QrModel | null {
  if (made.has(text)) return made.get(text) ?? null;
  const model = buildModel(text);
  made.set(text, model);
  if (made.size > MADE_KEPT) made.delete(made.keys().next().value as string);
  return model;
}

function buildModel(text: string): QrModel | null {
  const encoded = encodeStrongest(text);
  if (!encoded) return null;
  const { level, qr } = encoded;
  const { size, data: dark, types } = qr;
  const knockout = knockoutFor(size, types, level);
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

  return { level, size, knockout, logo, modules: parts.join(''), finders, dark, types };
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
