import { describe, expect, it } from 'vitest';
import type { BackendChatMessage, ScrollbarMarkStamp } from '../../services/twitchChat';
import { collectMarks, layoutTicks, type PlacedMark } from './scrollbarMarkLayout';

const msg = (id: string, mark?: ScrollbarMarkStamp) =>
  ({ id, metadata: mark ? { scrollbar_mark: mark } : {} }) as unknown as BackendChatMessage;
const idOf = (m: string | BackendChatMessage) => (typeof m === 'string' ? null : m.id);
const ALL = new Set(['mention', 'highlight', 'pinned', 'announcement', 'sub', 'redeem', 'raid', 'first_time']);

/** Stacks rows of the given heights from `top`, as the list lays them out. */
function stack(heights: number[], marks: Record<number, PlacedMark['kind']>, top = 0): PlacedMark[] {
  const out: PlacedMark[] = [];
  let y = top;
  heights.forEach((h, i) => {
    if (marks[i]) out.push({ kind: marks[i], top: y, height: h });
    y += h;
  });
  return out;
}

describe('collectMarks', () => {
  it('keeps the stamped kinds the viewer shows, replies under mentions', () => {
    const marks = collectMarks(
      [msg('a', { kind: 'mention', color: '#f00' }), msg('b', { kind: 'reply' }), msg('c', { kind: 'first_time' }), msg('d'), 'raw line'],
      idOf,
      new Set(['mention', 'sub']),
    );
    expect([...marks.keys()]).toEqual(['a', 'b']);
    expect(marks.get('a')).toEqual({ kind: 'mention', color: '#f00' });
    expect(marks.get('b')).toEqual({ kind: 'reply' });
  });

  it('marks pinned rows, below a mention but above everything else', () => {
    const pinned = new Set(['a', 'b', 'c']);
    const marks = collectMarks(
      [msg('a', { kind: 'mention' }), msg('b', { kind: 'sub' }), msg('c'), msg('d', { kind: 'sub' })],
      idOf,
      ALL,
      pinned,
    );
    expect(marks.get('a')?.kind).toBe('mention');
    expect(marks.get('b')?.kind).toBe('pinned');
    expect(marks.get('c')?.kind).toBe('pinned');
    expect(marks.get('d')?.kind).toBe('sub');

    // Pinned switched off: the row keeps its own stamp, a plain row has none.
    const off = collectMarks([msg('b', { kind: 'sub' }), msg('c')], idOf, new Set(['sub']), pinned);
    expect(off.get('b')?.kind).toBe('sub');
    expect(off.has('c')).toBe(false);
  });

  it('marks nothing when every kind is off', () => {
    expect(collectMarks([msg('a', { kind: 'mention' })], idOf, new Set(), new Set(['a'])).size).toBe(0);
  });

  it('follows the buffer as old rows trim off the top', () => {
    const feed = [msg('a', { kind: 'sub' }), msg('b'), msg('c', { kind: 'mention' }), msg('d')];
    expect([...collectMarks(feed, idOf, ALL).keys()]).toEqual(['a', 'c']);
    const trimmed = feed.slice(2).concat(msg('e', { kind: 'raid' }));
    expect([...collectMarks(trimmed, idOf, ALL).keys()]).toEqual(['c', 'e']);
  });
});

describe('layoutTicks', () => {
  it('places ticks by pixel position with variable row heights', () => {
    // Rows of 20, 100, 20, 60 px: the third row starts at 120 of 200 px.
    const rows = stack([20, 100, 20, 60], { 0: 'mention', 2: 'sub' });
    const ticks = layoutTicks(rows, 200, 100);
    expect(ticks).toHaveLength(2);
    const byKind = Object.fromEntries(ticks.map((t) => [t.kind, t]));
    // Row 0 spans 0-20 → track 0-10; a 5px max tick centred on 5.
    expect(byKind.mention).toMatchObject({ y: 3, h: 5 });
    // Row 2 spans 120-140 → track 60-70; centred on 65.
    expect(byKind.sub).toMatchObject({ y: 63, h: 5 });
  });

  it('keeps a tiny row visible at the minimum height and inside the track', () => {
    const ticks = layoutTicks([{ kind: 'raid', top: 9990, height: 10 }], 10000, 300);
    expect(ticks).toHaveLength(1);
    expect(ticks[0].h).toBe(3);
    expect(ticks[0].y + ticks[0].h).toBeLessThanOrEqual(300);
  });

  it('merges a dense run of one kind into one bar, the stronger kind on top', () => {
    // A 50-row gift bomb of 20px rows with a mention in the middle, on a track
    // a tenth of the content: the subs become one run, the mention stays its
    // own tick and comes last so it paints over the run.
    const marks: Record<number, PlacedMark['kind']> = {};
    for (let i = 0; i < 50; i++) marks[i] = i === 25 ? 'mention' : 'sub';
    const ticks = layoutTicks(stack(Array(50).fill(20), marks, 1000), 10000, 1000);
    expect(ticks).toHaveLength(2);
    // Rows span track 100-200; each tick is the 3px minimum centred on its row.
    expect(ticks[0]).toMatchObject({ kind: 'sub', y: 100, h: 101 });
    expect(ticks[1].kind).toBe('mention');
    expect(ticks[1].y).toBeGreaterThan(140);
    expect(ticks[1].y).toBeLessThan(160);
  });

  it('never lets a neighbour of another kind recolour a run', () => {
    const rows: PlacedMark[] = [
      { kind: 'highlight', color: '#0f8', top: 0, height: 10 },
      { kind: 'sub', top: 10, height: 10 },
      { kind: 'sub', top: 20, height: 10 },
      { kind: 'sub', top: 30, height: 10 },
    ];
    const ticks = layoutTicks(rows, 1000, 100);
    expect(ticks.map((t) => t.kind)).toEqual(['sub', 'highlight']);
    expect(ticks[0].color).toBeUndefined();
    expect(ticks[1].color).toBe('#0f8');
  });

  it('merges a run only within one colour of a kind', () => {
    // Two highlight rules side by side, then a second row of the first rule.
    const rows: PlacedMark[] = [
      { kind: 'highlight', color: '#0f8', top: 0, height: 10 },
      { kind: 'highlight', color: '#f80', top: 10, height: 10 },
      { kind: 'highlight', color: '#0f8', top: 20, height: 10 },
    ];
    const ticks = layoutTicks(rows, 1000, 100);
    expect(ticks.map((t) => t.color).sort()).toEqual(['#0f8', '#f80']);
    // The two green rows merged into one run; the orange one kept its own tick.
    const green = ticks.find((t) => t.color === '#0f8')!;
    const orange = ticks.find((t) => t.color === '#f80')!;
    expect(green.h).toBeGreaterThan(orange.h);
    // Same kind and colour still merge, and an uncoloured run stays apart.
    const plain = layoutTicks(
      [
        { kind: 'sub', top: 0, height: 10 },
        { kind: 'sub', top: 10, height: 10 },
        { kind: 'sub', color: '#a855f7', top: 20, height: 10 },
      ],
      1000,
      100,
    );
    expect(plain).toHaveLength(2);
  });

  it('bounds the tick count by the track height per kind', () => {
    const marks: Record<number, PlacedMark['kind']> = {};
    for (let i = 0; i < 2000; i += 2) marks[i] = i % 4 ? 'sub' : 'mention';
    const ticks = layoutTicks(stack(Array(2000).fill(24), marks), 48000, 400);
    expect(ticks.length).toBeLessThanOrEqual(2 * (400 / 4));
    for (const kind of ['sub', 'mention']) {
      const own = ticks.filter((t) => t.kind === kind);
      for (let i = 1; i < own.length; i++) expect(own[i].y).toBeGreaterThan(own[i - 1].y + own[i - 1].h);
    }
  });

  it('shifts ticks up when rows trim from the top', () => {
    const before = layoutTicks(stack([30, 30, 30, 30], { 3: 'sub' }), 120, 120);
    const after = layoutTicks(stack([30, 30, 30], { 2: 'sub' }), 90, 120);
    expect(after[0].y).toBeLessThan(before[0].y);
    expect(after[0].y).toBe(Math.round(((60 + 15) / 90) * 120 - 2.5));
  });

  it('draws nothing for an empty or unscrollable list', () => {
    expect(layoutTicks([], 100, 100)).toEqual([]);
    expect(layoutTicks([{ kind: 'sub', top: 0, height: 10 }], 0, 100)).toEqual([]);
  });
});
