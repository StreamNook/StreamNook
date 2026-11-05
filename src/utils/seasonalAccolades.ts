// Seasonal accolades: earned by opening StreamNook inside a holiday or season
// window. An earned one is stored with the account (user_accolades) and stays
// collected after its window closes.
//
// Seasons are broad windows (the current season is collected just by being
// around) and holidays are narrow. A window may wrap the new year. Cake Day is
// the member's own Twitch account anniversary, so it has no fixed window and is
// decided by isCakeDay / isCakeDayOn. The list and each window come from
// accolades.catalog.json (entries of kind "seasonal").
import { CATALOG } from './accoladeCatalogData';

export interface SeasonalWindow {
  fromM: number; // month 1-12
  fromD: number;
  toM: number;
  toD: number;
}

export interface SeasonalAccoladeDef {
  id: string; // stable id stored in user_accolades.accolade_id
  label: string;
  // How to earn it, shown on the locked accolade's tooltip.
  hint: string;
  window: SeasonalWindow;
}

export const CAKE_DAY_ID = 'cake_day';

export const SEASONAL_ACCOLADES: SeasonalAccoladeDef[] = CATALOG.accolades.flatMap((a) =>
  a.kind === 'seasonal' && a.window && a.hint
    ? [{ id: a.id, label: a.label, hint: a.hint, window: { fromM: a.window.from_m, fromD: a.window.from_d, toM: a.window.to_m, toD: a.window.to_d } }]
    : [],
);

const inWindow = (month: number, day: number, w: SeasonalWindow): boolean => {
  const cur = month * 100 + day;
  const from = w.fromM * 100 + w.fromD;
  const to = w.toM * 100 + w.toD;
  // Windows that wrap the year boundary (e.g. winter, Dec 21 to Mar 19).
  return from <= to ? cur >= from && cur <= to : cur >= from || cur <= to;
};

/** Every seasonal accolade id whose window contains the month (1-12) and day (a
 *  day can hit a holiday AND its season, e.g. Halloween + Fall). */
export const seasonalIdsOn = (month: number, day: number): string[] =>
  SEASONAL_ACCOLADES.filter((b) => inWindow(month, day, b.window)).map((b) => b.id);

/** Every seasonal accolade id whose window contains `date`'s local day. */
export const getActiveSeasonalAccoladeIds = (date: Date): string[] => seasonalIdsOn(date.getMonth() + 1, date.getDate());

/** True when the local month and day of `createdAt` are the given month and day. */
export const isCakeDayOn = (createdAt: string, month: number, day: number): boolean => {
  const created = new Date(createdAt);
  if (Number.isNaN(created.getTime())) return false;
  return created.getMonth() + 1 === month && created.getDate() === day;
};

/** True when `date`'s month/day matches the Twitch account creation date. */
export const isCakeDay = (createdAt: string, date: Date): boolean => isCakeDayOn(createdAt, date.getMonth() + 1, date.getDate());

/** The month and day of a `YYYY-MM-DD` string, or null when it is not one. */
export const parseDay = (value: string): { month: number; day: number } | null => {
  const m = /^\d{4}-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const month = Number(m[1]);
  const day = Number(m[2]);
  return month >= 1 && month <= 12 && day >= 1 && day <= 31 ? { month, day } : null;
};
