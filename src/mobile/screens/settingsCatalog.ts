// The settings catalog Rust keeps (services/settings_catalog.rs): every settings
// row with its page, section, the paths it writes and the words it is searched
// by. Settings search and Changed settings both list its rows; this maps a row
// onto the phone's settings pages.
import { useMobileNavStore } from '../navStore';
import { Logger } from '../../utils/logger';

/** Mirrors Rust `settings_catalog::CatalogRow`. */
export interface CatalogRow {
  /** `player`, `adFree`, `chat`, `moderation`, `theme`, `notifications`, `cache`, `support`. */
  page: string;
  section: string;
  title: string;
  paths: string[];
  nested?: boolean;
  detail: string;
  aliases?: string[];
  /** The brand whose mark leads the title (`7tv`, `ffz`, `bttv`). */
  logo?: string | null;
  /** A nested row's parent row title (Rust fills it). */
  under?: string | null;
}

/** The catalog's page ids, as the settings layer names its pages. */
const PAGE_VIEW: Record<string, string> = {
  player: 'Player',
  adFree: 'Ad-free',
  chat: 'Chat',
  moderation: 'Moderation',
  theme: 'Theme',
  notifications: 'Notifications',
  cache: 'Cache',
  support: 'Support',
};

/** The order pages are listed in, the You tab's order. */
export const CATALOG_PAGES = Object.keys(PAGE_VIEW);

/** The settings page a catalog page id names, or null for a page this phone
 *  does not have. */
export const pageViewOf = (page: string): string | null => PAGE_VIEW[page] ?? null;

/** The rows this phone can draw: a row it cannot read, or one on a page it
 *  does not have, is logged and left out, so one row never blanks a list (the
 *  iPhone does the same). */
export function knownRows(rows: CatalogRow[], where: string): CatalogRow[] {
  return rows.filter((row) => {
    if (row && typeof row.title === 'string' && Array.isArray(row.paths) && pageViewOf(row.page)) return true;
    Logger.warn(`[${where}] settings row skipped:`, row);
    return false;
  });
}

/** "Chat › Message Layout": where the row lives. A section named like its page
 *  ("Cache" on Cache) says it once; a nested row adds the row it sits under. */
export function rowPlace(row: CatalogRow): string {
  const page = pageViewOf(row.page) ?? row.page;
  const place = row.section && row.section.toLowerCase() !== page.toLowerCase() ? `${page} › ${row.section}` : page;
  return row.under ? `${place} › ${row.under}` : place;
}

/** A row's identity: two rows can share a title, never a title and paths. */
export const catalogRowKey = (row: CatalogRow): string =>
  `${row.page}|${row.section}|${row.title}|${row.paths.join(',')}`;

/** Opens the page a row lives on and lands on its section (and on the row,
 *  where the page draws one by this title). */
export function openCatalogRow(row: CatalogRow): void {
  const view = pageViewOf(row.page);
  if (!view) return;
  useMobileNavStore.getState().openSettings(view, { section: row.section, title: row.title });
}
