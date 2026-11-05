// getEmoteUsageSummary reads a member's emote totals exactly: the count column of
// every row, a thousand rows a page in a stable order, instead of one read that
// stops at the first thousand. This feeds the real function a fake table.
import { beforeEach, describe, expect, it, vi } from 'vitest';

interface Row {
  twitch_user_id: string;
  emote_id: string;
  emote_name: string;
  provider: string;
  image_url: string | null;
  count: number;
}

const db = vi.hoisted(() => ({
  rows: [] as Row[],
  /** Every read the function made, as the query the fake saw. */
  reads: [] as Array<{ columns: string; user: string | null; order: string[]; limit: number | null; range: [number, number] | null }>,
  failPage: -1,
}));

vi.mock('@supabase/supabase-js', () => {
  class Query {
    private read = { columns: '', user: null as string | null, order: [] as string[], limit: null as number | null, range: null as [number, number] | null };
    select(columns: string) {
      this.read.columns = columns;
      return this;
    }
    eq(_column: string, value: string) {
      this.read.user = value;
      return this;
    }
    order(column: string, opts: { ascending: boolean }) {
      this.read.order.push(`${column}.${opts.ascending ? 'asc' : 'desc'}`);
      return this;
    }
    limit(n: number) {
      this.read.limit = n;
      return this;
    }
    range(from: number, to: number) {
      this.read.range = [from, to];
      return this;
    }
    then(resolve: (v: { data: unknown; error: { message: string } | null }) => void) {
      db.reads.push({ ...this.read });
      const mine = db.rows.filter((r) => r.twitch_user_id === this.read.user);
      const byCount = this.read.order[0] === 'count.desc';
      mine.sort((a, b) => (byCount ? b.count - a.count : 0) || (a.emote_id < b.emote_id ? -1 : 1));
      const range = this.read.range;
      if (range && db.failPage >= 0 && Math.floor(range[0] / 1000) === db.failPage) return resolve({ data: null, error: { message: 'boom' } });
      const slice = range ? mine.slice(range[0], range[1] + 1) : mine.slice(0, this.read.limit ?? mine.length);
      const columns = this.read.columns.split(',').map((c) => c.trim());
      resolve({ data: slice.map((r) => Object.fromEntries(columns.map((c) => [c, r[c as keyof Row]]))), error: null });
    }
  }
  return { createClient: () => ({ from: () => new Query() }), RealtimeChannel: class {}, SupabaseClient: class {} };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.stubEnv('VITE_SUPABASE_URL', 'https://example.supabase.co');
vi.stubEnv('VITE_SUPABASE_ANON_KEY', 'anon');

const table = (n: number, user = 'u1'): Row[] =>
  Array.from({ length: n }, (_, i) => ({
    twitch_user_id: user,
    emote_id: `e${String(i).padStart(5, '0')}`,
    emote_name: `Emote${i}`,
    provider: '7tv',
    image_url: null,
    count: (i % 7) + 1,
  }));

const load = async () => (await import('./supabaseService')).getEmoteUsageSummary;

describe('getEmoteUsageSummary', () => {
  beforeEach(() => {
    db.rows = [];
    db.reads = [];
    db.failPage = -1;
  });

  it('counts every emote of a member with more than a thousand', async () => {
    db.rows = [...table(2500), ...table(40, 'someone-else')];
    const summary = await (await load())('u1', 12);
    expect(summary.uniqueCount).toBe(2500);
    expect(summary.totalCount).toBe(db.rows.filter((r) => r.twitch_user_id === 'u1').reduce((s, r) => s + r.count, 0));
    expect(summary.top).toHaveLength(12);
    expect(summary.top.every((e) => e.count === 7)).toBe(true);
  });

  it('pages the count column alone, a thousand rows at a time, in emote_id order', async () => {
    db.rows = table(2500);
    await (await load())('u1', 12);
    const pages = db.reads.filter((r) => r.range);
    expect(pages.map((p) => p.range)).toEqual([[0, 999], [1000, 1999], [2000, 2999]]);
    for (const p of pages) {
      expect(p.columns).toBe('count');
      expect(p.order).toEqual(['emote_id.asc']);
      expect(p.user).toBe('u1');
    }
  });

  it('asks again after an exactly full page, and stops on the short one after it', async () => {
    db.rows = table(1000);
    const summary = await (await load())('u1', 12);
    expect(summary.uniqueCount).toBe(1000);
    expect(db.reads.filter((r) => r.range)).toHaveLength(2);
  });

  it('answers zeros for a member with no rows, and for nobody', async () => {
    const run = await load();
    expect(await run('u1', 12)).toEqual({ top: [], uniqueCount: 0, totalCount: 0 });
    expect(await run('', 12)).toEqual({ top: [], uniqueCount: 0, totalCount: 0 });
  });

  it('answers zeros when a page fails, never a short total', async () => {
    db.rows = table(2500);
    db.failPage = 1;
    expect(await (await load())('u1', 12)).toEqual({ top: [], uniqueCount: 0, totalCount: 0 });
  });
});
