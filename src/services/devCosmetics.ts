// Dev-only badge slots, the badge counterpart of devAtmospheres.ts. A badge
// normally ships as a `cosmetics` row plus an R2 asset, and the first time
// anyone sees it in the REAL app is after it is live for every user. These
// entries merge into the catalog on dev builds only (import.meta.env.DEV),
// count as owned, and can be equipped without touching the server (the equip
// endpoint would refuse an unowned slug), so a candidate badge can be judged
// in real chat rows, the picker, and the hover card before its row exists.
// Production builds never see this file's contents.
//
// Assets are served by the Vite dev server from the untracked dev-preview/
// folder at the repo root, so nothing here reaches a bundle either.
import type { CosmeticCatalogEntry } from './supabaseService';

// Empty between candidates. A dev entry REPLACES a live row of the same slug
// on dev builds and counts as owned, so remove an entry the moment its real
// row exists (Facet and Lumen shipped 2026-09-02 and were removed here the
// same day). Shape of an entry: the `CosmeticCatalogEntry` exactly as its
// future `cosmetics` row. asset_path must be an absolute http(s) URL:
// resolveCosmeticAsset drops anything else and the badge would silently never
// render; a not-yet-uploaded candidate can use
// `${location.origin}/dev-preview/<file>` served by the Vite dev server.
export const DEV_COSMETICS: CosmeticCatalogEntry[] = [];

const DEV_SLUGS = new Set(DEV_COSMETICS.map((c) => c.slug));

/** True for a dev-only slot, so the catalog treats it as owned and equips it
 *  locally on dev builds. Always false in production (the set is unreachable). */
export const isDevCosmetic = (slug: string | null | undefined): boolean =>
  !!import.meta.env.DEV && !!slug && DEV_SLUGS.has(slug);

// A dev badge equipped locally survives a reload through localStorage, since
// the server never hears about it.
const ACTIVE_KEY = 'sn_dev_active_cosmetic_v1';

export const readDevActiveCosmetic = (): { userId: string; slug: string } | null => {
  if (!import.meta.env.DEV) return null;
  try {
    const raw = localStorage.getItem(ACTIVE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { userId?: unknown; slug?: unknown };
    if (typeof parsed.userId === 'string' && typeof parsed.slug === 'string' && DEV_SLUGS.has(parsed.slug)) {
      return { userId: parsed.userId, slug: parsed.slug };
    }
  } catch { /* ignore */ }
  return null;
};

export const writeDevActiveCosmetic = (userId: string, slug: string | null): void => {
  if (!import.meta.env.DEV) return;
  try {
    if (slug) localStorage.setItem(ACTIVE_KEY, JSON.stringify({ userId, slug }));
    else localStorage.removeItem(ACTIVE_KEY);
  } catch { /* ignore */ }
};
