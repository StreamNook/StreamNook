import { describe, expect, it } from 'vitest';
import { CAKE_DAY_ID, SEASONAL_ACCOLADES, getActiveSeasonalAccoladeIds, isCakeDay, isCakeDayOn, parseDay, seasonalIdsOn } from './seasonalAccolades';

describe('seasonal windows', () => {
  it('lists the twelve holidays and seasons from the catalog', () => {
    expect(SEASONAL_ACCOLADES.map((a) => a.id)).toEqual([
      'new_year', 'valentines', 'st_patricks', 'april_fools', 'may_fourth', 'halloween', 'thanksgiving', 'winter_holiday',
      'spring', 'summer', 'fall', 'winter',
    ]);
    expect(CAKE_DAY_ID).toBe('cake_day');
  });

  it('holds a window on both of its edge days and not on the days beside them', () => {
    expect(seasonalIdsOn(2, 12)).not.toContain('valentines');
    expect(seasonalIdsOn(2, 13)).toContain('valentines');
    expect(seasonalIdsOn(2, 15)).toContain('valentines');
    expect(seasonalIdsOn(2, 16)).not.toContain('valentines');
  });

  it('carries a window across the new year', () => {
    expect(seasonalIdsOn(12, 30)).not.toContain('new_year');
    expect(seasonalIdsOn(12, 31)).toContain('new_year');
    expect(seasonalIdsOn(1, 1)).toEqual(expect.arrayContaining(['new_year', 'winter']));
    expect(seasonalIdsOn(1, 2)).not.toContain('new_year');
    expect(seasonalIdsOn(3, 19)).toContain('winter');
    expect(seasonalIdsOn(3, 20)).not.toContain('winter');
  });

  it('puts a day in a holiday and its season together', () => {
    expect(getActiveSeasonalAccoladeIds(new Date(2026, 9, 31)).sort()).toEqual(['fall', 'halloween']);
  });
});

describe('cake day', () => {
  it('matches the month and day of the creation date', () => {
    expect(isCakeDayOn('2014-03-20T12:00:00Z', 3, 20)).toBe(true);
    expect(isCakeDayOn('2014-03-20T12:00:00Z', 3, 21)).toBe(false);
    expect(isCakeDayOn('2014-03-20T12:00:00Z', 4, 20)).toBe(false);
    expect(isCakeDay('2014-03-20T12:00:00Z', new Date(2026, 2, 20))).toBe(true);
  });

  it('is never a cake day without a readable creation date', () => {
    expect(isCakeDayOn('not a date', 3, 20)).toBe(false);
    expect(isCakeDay('', new Date(2026, 2, 20))).toBe(false);
  });
});

describe('parseDay', () => {
  it('reads a YYYY-MM-DD day', () => {
    expect(parseDay('2026-11-05')).toEqual({ month: 11, day: 5 });
  });

  it('refuses anything else', () => {
    for (const bad of ['', 'today', '2026-11-5', '2026-13-01', '2026-00-10', '2026-11-00', '2026-11-32', '11/05/2026']) {
      expect(parseDay(bad), bad).toBeNull();
    }
  });
});
