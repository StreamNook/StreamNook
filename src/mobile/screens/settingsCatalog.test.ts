// One settings row this phone cannot draw never blanks a list: Changed
// settings and search keep every row they know and log the rest.
import { describe, expect, it, vi } from 'vitest';

vi.mock('../navStore', () => ({ useMobileNavStore: { getState: () => ({ openSettings: vi.fn() }) } }));
const warn = vi.fn();
vi.mock('../../utils/logger', () => ({ Logger: { warn: (...args: unknown[]) => warn(...args) } }));

import { knownRows, type CatalogRow } from './settingsCatalog';

const row = (page: string, title: string): CatalogRow => ({ page, section: 'S', title, paths: ['x.y'], detail: '' });

describe('knownRows', () => {
  it('keeps every row on a page this phone has and logs the others', () => {
    const rows = [
      row('chat', 'Text size'),
      row('somewhere-new', 'Future row'),
      row('moderation', 'Steps'),
      { page: 'chat', section: 'S' } as unknown as CatalogRow,
    ];
    expect(knownRows(rows, 'Test').map((r) => r.title)).toEqual(['Text size', 'Steps']);
    expect(warn).toHaveBeenCalledTimes(2);
  });
});
