// StreamNook Atmospheres: our own line of profile backdrops. Each one is
// StreamNook IP (a designed gradient scene or a pre-rendered image), so it's
// clean to sell. An Atmosphere themes the profile background AND signs the
// member's chat messages with a matching wash, so the cosmetic follows them
// everywhere they show up.
//
// The catalog is SERVER-DRIVEN: definitions live in the Supabase `atmospheres`
// table and are fetched into an in-memory registry at startup (see
// supabaseService.ts). So a new atmosphere ships as a DB row + an R2 asset, live
// for every client with no desktop release, and a viewer on an older build can
// still render a newer atmosphere. `getAtmosphere` / `listAtmospheres` below just
// read that registry. This file owns only the shared `Atmosphere` shape.

import { getAtmosphereEntry, listAtmosphereEntries } from './supabaseService';
import { MAJOR_COLOGNE_THEME_ID, isCologneTheme } from './cologneEvent';

export interface Atmosphere {
  id: string;
  name: string;
  // Accent color as "r, g, b" for borders / tiles / the chat accent bar.
  accent: string;
  // A representative CSS background for the picker swatch (small preview).
  swatch: string;
  // Solid base color (fixed; the animated layer(s) / image sit over it).
  baseColor: string;
  // Optional fixed gradients painted over the base (e.g. a top glow) that do NOT
  // move, so the motion reads against a steady backdrop.
  baseLayers?: string;
  // A pre-rendered image (e.g. an animated webp) used as the whole backdrop in
  // place of the CSS gradient layers. When set, the renderer shows this image
  // (it carries its own motion) and ignores `layers` / `layers2`.
  image?: string;
  // When a landscape image should read PORTRAIT in the tall profile panel,
  // render the profile backdrop rotated 90deg. The chat-row wash always stays
  // landscape (rows are wide and short, so the image fits as-is there).
  imageProfilePortrait?: boolean;
  // The animated CSS layer (a nebula, or one set of aurora curtains). Optional:
  // image-backed atmospheres omit it.
  layers?: string;
  // Optional second layer (e.g. a second aurora curtain set flowing the other
  // way at a different speed, which is what sells the organic motion).
  layers2?: string;
  // Transform-only motion: 'drift' eases a nebula around; 'aurora' continuously
  // flows two curtain layers horizontally (seamless loop) with a gentle sway.
  motion: 'drift' | 'aurora';
  // A 1px gradient edge drawn down the left of the member's chat message row (in
  // place of a flat accent bar). 'none' draws no edge at all, for atmospheres
  // whose wash is its own signature and wants no bar beside it.
  chatEdge: string;
  // Frosted readability block (dark translucent fill + slight backdrop blur)
  // behind the member's chat text. Only for atmospheres whose wash is busy
  // enough to fight the text (typically image-backed ones); subtle gradient
  // washes leave it off.
  chatFrost?: boolean;
  // Defocus the image wash behind the chat row by this many px, so a crisp
  // asset reads as dispersed light on the row rather than a cropped picture.
  // Image-backed atmospheres only; gradient washes ignore it.
  chatBlur?: number;
  // A 1px rim around the member's chat row, as a CSS gradient, so the wash
  // reads as light caught on the row's glass edge. Absent = no rim.
  chatRim?: string;
  // How this Atmosphere is unlocked. 'subscriber' = the paid subscriber tier
  // (the default for the house line). 'accolade' = earned by unlocking a
  // specific accolade, available to ANY member regardless of subscription.
  // Absent is treated as 'subscriber' by the gate, for safety.
  unlock?: { kind: 'subscriber' } | { kind: 'accolade'; accoladeId: string };
  // Custom render kind. Absent = the standard gradient / image backdrop described
  // above. 'cologne-chrome' = the CS2 Major Cologne decoration (animated glass
  // wash + optional coin watermark + optional nine-slice gold frame), drawn by
  // MajorCologneChrome from the chrome* URLs below instead of `layers` / `image`.
  // One system extended for a richer render type, not a second system.
  kind?: 'cologne-chrome';
  // R2 asset URLs for the 'cologne-chrome' kind (ignored by other kinds): the
  // seamless glass tile that scrolls, the medallion watermark, and the nine-slice
  // border. The coin / frame only paint when the viewer's add-on flags (parsed off
  // the theme id, see cologneEvent) enable them; the texture always paints.
  chromeTexture?: string;
  chromeCoin?: string;
  chromeFrame?: string;
}

// Sync reads of the fetched catalog. Return null / empty until the registry has
// loaded (consumers re-render via the version subscription); see supabaseService.
//
// The CS2 Major Cologne look is a catalog row like any other (kind:'cologne-chrome'
// with R2 assets). Its APPLIED theme id can carry add-on suffix flags ('+coin' /
// '+border', see cologneEvent), but the registry keys the row by its base id, so
// strip the suffix to find the row and carry the FULL id through, so consumers
// (AtmosphereBackground) can read the add-on flags back off atm.id.
export const getAtmosphere = (id: string | null | undefined): Atmosphere | null => {
  if (isCologneTheme(id)) {
    const base = getAtmosphereEntry(MAJOR_COLOGNE_THEME_ID);
    return base ? { ...base, id: id! } : null;
  }
  return getAtmosphereEntry(id);
};

export const listAtmospheres = (): Atmosphere[] => listAtmosphereEntries();

// Public unlock descriptions, keyed by the accolade that grants the atmosphere.
// Anything accolade-gated NOT listed here is treated as a HIDDEN challenge: the
// library shows it exists but never reveals how it is earned. Safe by default,
// so a new secret atmosphere stays a secret until it is deliberately made public.
const PUBLIC_ATMOSPHERE_UNLOCKS: Record<string, string> = {
  semiquincentennial_2026: 'Semiquincentennial',
  cs2_major_cologne_2026: 'CS2 Major Cologne',
};

export interface AtmosphereUnlock {
  // 'subscriber' = paid tier reward; 'accolade' = earned via a badge.
  kind: 'subscriber' | 'accolade';
  // A secret challenge whose earn method must not be revealed.
  hidden: boolean;
  // The accolade's display name for a public accolade unlock (absent when
  // hidden). An accolade is an achievement medallion on the profile wall, not a
  // chat badge; user-facing copy must say "accolade".
  accoladeName?: string;
  // One-line status for the library / tooltips.
  label: string;
}

// How an atmosphere is obtained, for the library and any "how did I get this"
// surface. Keeps the hidden-challenge rule in one place.
export const getAtmosphereUnlock = (a: Atmosphere): AtmosphereUnlock => {
  if (a.unlock?.kind === 'accolade') {
    const accoladeName = PUBLIC_ATMOSPHERE_UNLOCKS[a.unlock.accoladeId];
    return accoladeName
      ? { kind: 'accolade', hidden: false, accoladeName, label: `Earned with the ${accoladeName} accolade` }
      : { kind: 'accolade', hidden: true, label: 'Hidden challenge' };
  }
  return { kind: 'subscriber', hidden: false, label: 'Subscriber reward' };
};
