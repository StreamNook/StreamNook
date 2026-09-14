// Dev-only atmosphere slots. A cosmetic normally ships as a Supabase row plus
// an R2 asset, which means the first time anyone sees it in the REAL app is
// after it is already live for every user. These entries merge into the
// catalog on dev builds only (import.meta.env.DEV), so a candidate asset can be
// picked in Profile > Customize and judged in real chat rows, the real profile
// panel, and the real hover card before a row is ever inserted. Production
// builds never see this file's contents.
//
// The asset is served by the Vite dev server from the untracked dev-preview/
// folder at the repo root, so nothing here reaches a bundle either.
import type { Atmosphere } from './atmospheres';

// Empty between candidates. A dev entry REPLACES a live row of the same id on
// dev builds, so remove an entry the moment its real row exists (Dispersion
// shipped 2026-09-02 and was removed here the same day). Shape of an entry:
// the `Atmosphere` object exactly as its future `atmospheres` row, with
// `image` pointing at `${location.origin}/dev-preview/<file>` served by the
// Vite dev server until the asset is on the CDN.
export const DEV_ATMOSPHERES: Atmosphere[] = [];

const DEV_IDS = new Set(DEV_ATMOSPHERES.map((a) => a.id));

/** True for a dev-only slot, so the picker can show it unlocked on dev builds. */
export const isDevAtmosphere = (id: string | null | undefined): boolean =>
  !!import.meta.env.DEV && !!id && DEV_IDS.has(id);
