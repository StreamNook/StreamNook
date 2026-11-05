// The cases in accolades.cases.json are only as good as what they reach. This
// reads the catalog and the cases and fails when a catalog accolade is never
// seen earned or locked, a threshold is never tried one short and exactly on,
// a hidden stat is never tried as both kinds of key, or the site's projection
// drifts from the full wall.
import { describe, expect, it } from 'vitest';
import cases from './accolades.cases.json';
import catalog from './accolades.catalog.json';

interface Item {
  id: string;
  label: string;
  earned: boolean;
  secret: boolean;
  hint: string;
}
interface Def {
  id: string;
  kind: string;
  ended?: boolean;
  rule: { stat?: string; n?: number; special?: string };
  progress?: string;
  goal: string;
  hint?: string;
}
interface Case {
  name: string;
  own: boolean;
  input: Record<string, number | string | null | string[]> & { saved: string[]; hidden: string[] };
  expect: { earned: number; total: number; items: Item[] };
  site?: Array<{ id: string; label: string; earned: boolean; hint: string }>;
}

const file = cases as unknown as { cases: Case[] };
const defs = (catalog as unknown as { accolades: Def[] }).accolades;
const titles = (catalog as unknown as { titles: Record<string, string> }).titles;
const all = file.cases;
const item = (c: Case, id: string): Item | undefined => c.expect.items.find((i) => i.id === id);

/** Never listed while locked: the number decides them, or the event is over. */
const neverLocked = (d: Def): boolean => !!d.ended || ['first_hundred', 'palindrome', 'nice'].includes(d.rule.special ?? '');

describe('accolade cases cover the catalog', () => {
  it('names every case once', () => {
    const names = all.map((c) => c.name);
    expect(new Set(names).size).toBe(names.length);
    expect(all.length).toBeGreaterThanOrEqual(16);
  });

  it('keeps every wall sorted earned first, with unique ids and honest counts', () => {
    for (const c of all) {
      const ids = c.expect.items.map((i) => i.id);
      expect(new Set(ids).size, c.name).toBe(ids.length);
      expect(c.expect.total, c.name).toBe(ids.length);
      expect(c.expect.earned, c.name).toBe(c.expect.items.filter((i) => i.earned).length);
      const firstLocked = c.expect.items.findIndex((i) => !i.earned);
      if (firstLocked >= 0) expect(c.expect.items.slice(firstLocked).every((i) => !i.earned), c.name).toBe(true);
    }
  });

  for (const d of defs) {
    it(`${d.id} is seen earned in some case`, () => {
      expect(all.some((c) => item(c, d.id)?.earned === true)).toBe(true);
    });
    it(neverLocked(d) ? `${d.id} is seen left off the wall` : `${d.id} is seen locked in some case`, () => {
      if (neverLocked(d)) expect(all.some((c) => !item(c, d.id))).toBe(true);
      else expect(all.some((c) => item(c, d.id)?.earned === false)).toBe(true);
    });
  }

  for (const d of defs.filter((x) => x.rule.stat)) {
    const stat = d.rule.stat as string;
    const n = d.rule.n as number;
    it(`${d.id} is tried at ${stat} = ${n - 1} and ${n}`, () => {
      const below = all.filter((c) => c.input[stat] === n - 1 && !c.input.saved.includes(d.id));
      const on = all.filter((c) => c.input[stat] === n && !c.input.saved.includes(d.id));
      expect(below.length).toBeGreaterThan(0);
      expect(on.length).toBeGreaterThan(0);
      for (const c of below) expect(item(c, d.id)?.earned, c.name).toBe(false);
      for (const c of on) expect(item(c, d.id)?.earned, c.name).toBe(true);
    });
  }
});

describe('hidden stats and the public hints', () => {
  const HIDES = [
    { stat: 'messages', stats: ['messages'], section: 'lifetime' },
    { stat: 'hours', stats: ['hours'], section: 'roast' },
    { stat: 'streams', stats: ['streams'], section: 'lifetime' },
    { stat: 'cosmetics', stats: ['cosmetics'], section: 'lifetime' },
    { stat: 'years', stats: ['years'], section: 'twitch' },
  ];
  const hideKey = (stat: string) => (stat === 'years' ? 'twitch_age' : stat);
  const sectionStats: Record<string, string[]> = {
    lifetime: ['messages', 'streams', 'favorite_channel', 'subs_gifted', 'cosmetics', 'member_rank'],
    roast: ['hours', 'roast_line'],
    twitch: ['twitch_age', 'followers', 'account_type'],
  };

  for (const h of HIDES) {
    const base = defs.filter((d) => d.kind === 'base' && d.rule.stat === h.stat);
    it(`${h.stat}: the goal alone, hidden by its own key`, () => {
      const seen = all.filter((c) => !c.own && c.input.hidden.includes(hideKey(h.stat)));
      expect(seen.length).toBeGreaterThan(0);
      for (const c of seen) for (const d of base) expect(item(c, d.id)?.hint, c.name).toBe(d.goal);
    });
    it(`${h.stat}: the goal alone, hidden by its section key`, () => {
      const seen = all.filter(
        (c) => !c.own && c.input.hidden.includes(h.section) && !sectionStats[h.section].some((k) => c.input.hidden.includes(k)),
      );
      expect(seen.length).toBeGreaterThan(0);
      for (const c of seen) for (const d of base) expect(item(c, d.id)?.hint, c.name).toBe(d.goal);
    });
    it(`${h.stat}: the numbers when it is visible, and a section key beaten by a stat saved alone`, () => {
      const visible = all.filter((c) => !c.own && !c.input.hidden.includes(hideKey(h.stat)) && !c.input.hidden.includes(h.section));
      expect(visible.length).toBeGreaterThan(0);
      for (const c of visible) for (const d of base) expect(item(c, d.id)?.hint, c.name).toMatch(new RegExp(`^${d.progress}: [\\d,]+ / [\\d,]+$`));
      const beaten = all.filter(
        (c) =>
          !c.own &&
          c.input.hidden.includes(h.section) &&
          sectionStats[h.section].some((k) => c.input.hidden.includes(k)) &&
          !c.input.hidden.includes(hideKey(h.stat)),
      );
      for (const c of beaten) for (const d of base) expect(item(c, d.id)?.hint, c.name).toMatch(new RegExp(`^${d.progress}: `));
    });
  }

  it('a section key yields to a stat saved on its own in at least one case', () => {
    expect(all.some((c) => !c.own && c.input.hidden.includes('lifetime') && c.input.hidden.includes('messages'))).toBe(true);
  });

  it('never prints drops or channel points on a public wall', () => {
    const publicCases = all.filter((c) => !c.own);
    for (const c of publicCases) {
      for (const d of defs.filter((x) => x.rule.stat === 'drops' || x.rule.stat === 'points')) {
        expect(item(c, d.id)?.hint, c.name).toBe(d.goal);
      }
    }
    expect(publicCases.some((c) => (c.input.drops as number) > 0 && (c.input.points as number) > 0)).toBe(true);
  });

  it('prints them on an own wall', () => {
    const own = all.filter((c) => c.own);
    expect(own.some((c) => /^Drops claimed: \d/.test(item(c, 'dropgoblin')?.hint ?? ''))).toBe(true);
    expect(own.some((c) => /^Channel points held: [\d,]+ \/ 100,000$/.test(item(c, 'hoarder')?.hint ?? ''))).toBe(true);
  });

  it('keeps secrets off a public wall until earned, and hides how an earned one was won', () => {
    for (const c of all.filter((x) => !x.own)) {
      for (const i of c.expect.items.filter((x) => x.secret)) {
        expect(i.earned, c.name).toBe(true);
        expect(i.hint, c.name).toBe(titles.hidden_secret_hint);
      }
    }
  });

  it('shows a locked secret with its cryptic hint on an own wall', () => {
    const own = all.filter((c) => c.own);
    for (const id of ['completionist', 'triple', 'insomniac']) {
      const d = defs.find((x) => x.id === id) as Def;
      expect(own.some((c) => item(c, id)?.earned === false && item(c, id)?.hint === d.hint)).toBe(true);
    }
  });
});

describe('member numbers, seasons and the stored set', () => {
  const NAMES: Array<[number, string]> = [
    [42, 'The Answer'],
    [67, 'Six Seven'],
    [141, 'Going Dark'],
    [404, 'Not Found'],
    [9000, 'Over 9000'],
    [1167, 'Six Seven-ish'],
  ];
  for (const [n, name] of NAMES) {
    it(`member ${n} reads ${name}`, () => {
      const seen = all.filter((c) => c.input.member_number === n);
      expect(seen.length).toBeGreaterThan(0);
      for (const c of seen) expect(item(c, 'nice'), c.name).toMatchObject({ label: name, earned: true });
    });
  }

  it('leaves Nice off the wall for a number that is not a meme', () => {
    const seen = all.filter((c) => c.input.member_number === 500);
    expect(seen.length).toBeGreaterThan(0);
    for (const c of seen) expect(item(c, 'nice'), c.name).toBeUndefined();
  });

  it('lights Completionist for a member past 100 without OG, and not one base short', () => {
    const held = defs.filter((d) => d.kind === 'base' && d.id !== 'og').map((d) => d.id);
    const all16 = all.filter((c) => (c.input.member_number as number) > 100 && held.every((id) => c.input.saved.includes(id)));
    expect(all16.length).toBeGreaterThan(0);
    for (const c of all16) expect(item(c, 'completionist')?.earned, c.name).toBe(true);
    const short = all.filter(
      (c) => !c.own && (c.input.member_number as number) > 100 && held.filter((id) => c.input.saved.includes(id)).length === held.length - 1,
    );
    expect(short.length).toBeGreaterThan(0);
    for (const c of short) expect(item(c, 'completionist'), c.name).toBeUndefined();
  });

  it('leaves OG and Palindrome off the wall for a number that is out of reach', () => {
    const seen = all.filter((c) => c.input.member_number === 168);
    expect(seen.length).toBeGreaterThan(0);
    for (const c of seen) {
      expect(item(c, 'og'), c.name).toBeUndefined();
      expect(item(c, 'palindrome'), c.name).toBeUndefined();
    }
  });

  it('keeps an ended event only for a member who holds it', () => {
    const events = defs.filter((d) => d.ended).map((d) => d.id);
    for (const c of all) {
      for (const id of events) {
        const held = c.input.saved.includes(id);
        expect(item(c, id) !== undefined, `${c.name}: ${id}`).toBe(held);
      }
    }
    expect(all.some((c) => events.every((id) => c.input.saved.includes(id)))).toBe(true);
  });

  it('lights a season from today on an own wall only', () => {
    const seasonal = defs.filter((d) => d.kind === 'seasonal').map((d) => d.id);
    const lit = all.filter((c) => c.own && seasonal.some((id) => item(c, id)?.earned && !c.input.saved.includes(id)));
    expect(lit.length).toBeGreaterThan(0);
    const ignored = all.filter((c) => !c.own && c.input.today === '2026-12-25');
    expect(ignored.length).toBeGreaterThan(0);
    for (const c of ignored) {
      for (const id of ['winter_holiday', 'winter']) expect(item(c, id)?.earned, c.name).toBe(false);
    }
  });

  it('lights Cake Day from the creation date on an own wall only', () => {
    expect(all.some((c) => c.own && item(c, 'cake_day')?.earned && !c.input.saved.includes('cake_day'))).toBe(true);
    const sameDay = all.filter(
      (c) => !c.own && c.input.today === '2026-12-25' && String(c.input.created_at).startsWith('2014-12-25') && !c.input.saved.includes('cake_day'),
    );
    expect(sameDay.length).toBeGreaterThan(0);
    for (const c of sameDay) expect(item(c, 'cake_day')?.earned, c.name).toBe(false);
  });

  it('says Collected for a stored season and a stored Cake Day', () => {
    for (const id of ['halloween', 'cake_day']) {
      const seen = all.filter((c) => c.input.saved.includes(id));
      expect(seen.length, id).toBeGreaterThan(0);
      for (const c of seen) expect(item(c, id)?.hint, `${c.name}: ${id}`).toBe(titles.collected);
    }
  });

  it('ignores a stored id the catalog does not know', () => {
    const seen = all.filter((c) => c.input.saved.includes('made_up_id'));
    expect(seen.length).toBeGreaterThan(0);
    for (const c of seen) expect(item(c, 'made_up_id'), c.name).toBeUndefined();
  });

  it('groups numbers of a thousand and over, and rounds hours', () => {
    expect(all.some((c) => item(c, 'chatterbox')?.hint === 'Messages sent: 1,234,567 / 1,000')).toBe(true);
    expect(all.some((c) => item(c, 'marathoner')?.hint === 'Hours watched: 12,346 / 100')).toBe(true);
    expect(all.some((c) => item(c, 'collector')?.hint === 'Cosmetics collected: 1,234 / 5')).toBe(true);
    expect(all.some((c) => item(c, 'nosleep')?.hint === 'Hours watched: 1,000 / 1,000' && item(c, 'nosleep')?.earned === false)).toBe(true);
  });

  it('runs a case with no member number', () => {
    expect(all.some((c) => c.input.member_number === null)).toBe(true);
  });
});

describe('the site projection', () => {
  const project = (c: Case) => c.expect.items.map((i) => ({ id: i.id, label: i.label, earned: i.earned, hint: i.hint }));

  it('is the full wall cut down to id, label, earned and hint, for every public case', () => {
    for (const c of all.filter((x) => !x.own)) expect(c.site, c.name).toStrictEqual(project(c));
  });

  it('is left off an own case, which the site never renders', () => {
    for (const c of all.filter((x) => x.own)) expect(c.site, c.name).toBeUndefined();
  });
});
