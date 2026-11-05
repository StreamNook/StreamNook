import { describe, expect, it } from 'vitest';
import raw from './accolades.catalog.json';
import { CATALOG, parseCatalog } from './accoladeCatalogData';

/** A deep copy of the real file to break one thing in. */
const copy = (): { accolades: Array<Record<string, unknown>>; [key: string]: unknown } => JSON.parse(JSON.stringify(raw));

describe('parseCatalog', () => {
  it('accepts the real catalog', () => {
    expect(CATALOG.accolades.length).toBe(raw.accolades.length);
    expect(CATALOG.accolades.map((a) => a.id)).toContain('chatterbox');
    expect(CATALOG.titles.hidden_secret_hint).toBe('A hidden accolade.');
  });

  it('refuses a key it does not know', () => {
    const bad = copy();
    bad.accolades[0].gradient = 'x';
    expect(() => parseCatalog(bad)).toThrow(/unknown key "gradient"/);
  });

  it('refuses a color that is not #rrggbb', () => {
    const bad = copy();
    bad.accolades[0].colors = ['#6aa9ff', 'blue'];
    expect(() => parseCatalog(bad)).toThrow(/colors/);
  });

  it('refuses a repeated id', () => {
    const bad = copy();
    bad.accolades[1].id = bad.accolades[0].id;
    expect(() => parseCatalog(bad)).toThrow(/duplicate id/);
  });

  it('refuses a seasonal accolade with no window, and a window on anything else', () => {
    const noWindow = copy();
    const season = noWindow.accolades.find((a) => a.kind === 'seasonal') as Record<string, unknown>;
    delete season.window;
    expect(() => parseCatalog(noWindow)).toThrow(/window/);
    const extra = copy();
    extra.accolades[0].window = { from_m: 1, from_d: 1, to_m: 1, to_d: 2 };
    expect(() => parseCatalog(extra)).toThrow(/window/);
  });

  it('refuses a base accolade whose progress label and rule disagree', () => {
    const bad = copy();
    delete bad.accolades[0].progress;
    expect(() => parseCatalog(bad)).toThrow(/progress/);
  });

  it('refuses a rule it does not know', () => {
    const bad = copy();
    bad.accolades[0].rule = { stat: 'friends', n: 3 };
    expect(() => parseCatalog(bad)).toThrow(/unknown stat/);
    bad.accolades[0].rule = { special: 'lucky' };
    expect(() => parseCatalog(bad)).toThrow(/unknown special/);
  });
});
