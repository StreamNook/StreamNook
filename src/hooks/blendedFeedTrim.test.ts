// Run with: npm test
//
// Two properties of the merged feed that are easy to get wrong and silent when
// you do. Both are modelled here as the pure array operations the hook performs,
// because the hook itself needs a live store and a React tree.

import { test } from 'vitest';
import assert from 'node:assert/strict';

/** The cap the hook applies to the COMBINED order array. */
function trimToCap(order: string[], held: Map<string, string>, cap: number) {
  let evicted = 0;
  if (order.length > cap) {
    const drop = order.length - cap;
    for (let i = 0; i < drop; i++) held.delete(order[i]);
    order.splice(0, drop);
    evicted = drop;
  }
  return evicted;
}

/** What the hook computes: scales with sources, stops at the ceiling. */
const mergedCap = (perSource: number, sources: number, ceiling = 400) =>
  Math.min(perSource * Math.max(1, sources), ceiling);

test('the cap scales with sources rather than flattening scrollback', () => {
  // A flat per-source cap would halve every reader's scrollback the moment they
  // combined a second platform.
  assert.equal(mergedCap(100, 1), 100);
  assert.equal(mergedCap(100, 2), 200);
  assert.equal(mergedCap(100, 3), 300);
  // ...but never past what an unwindowed list is comfortable mounting.
  assert.equal(mergedCap(1000, 3), 400, 'a raised buffer setting cannot blow the ceiling');
  assert.equal(mergedCap(100, 0), 100, 'no sources still yields a sane cap');
});

test('the merged feed caps in total, not per source', () => {
  // Three sources each holding a full buffer, capped to the combined ceiling.
  const cap = 100;
  const order: string[] = [];
  const held = new Map<string, string>();
  for (const src of ['tw', 'kick', 'yt']) {
    for (let i = 0; i < cap; i++) {
      const id = `${src}-${i}`;
      order.push(id);
      held.set(id, id);
    }
  }
  assert.equal(order.length, 300);

  const evicted = trimToCap(order, held, cap);
  assert.equal(order.length, cap, 'the combined array is what gets capped');
  assert.equal(evicted, 200);
  assert.equal(held.size, cap, 'and the held map shrinks with it, not just the order');
});

test('trimming takes the OLDEST rows, from the top', () => {
  const order = ['a', 'b', 'c', 'd', 'e'];
  const held = new Map(order.map((id) => [id, id]));
  trimToCap(order, held, 2);
  assert.deepEqual(order, ['d', 'e'], 'the newest survive');
  assert.equal(held.has('a'), false);
  assert.equal(held.has('e'), true);
});

test('the arrival counter keeps climbing past evictions', () => {
  // "N new since paused" anchors on this counter. Deriving it from the array
  // length alone would make it go BACKWARDS as the cap trims, so a paused reader
  // would watch the unread count fall while messages were still arriving.
  const order = ['a', 'b', 'c'];
  const held = new Map(order.map((id) => [id, id]));
  let evicted = 0;

  evicted += trimToCap(order, held, 2);
  const afterFirst = order.length + evicted;

  order.push('d', 'e');
  for (const id of ['d', 'e']) held.set(id, id);
  evicted += trimToCap(order, held, 2);
  const afterSecond = order.length + evicted;

  assert.ok(afterSecond > afterFirst, `counter must climb: ${afterFirst} -> ${afterSecond}`);
  assert.equal(afterSecond, 5, 'five messages have been seen in total');
});

test('a source joining later has its backlog absorbed, not appended', () => {
  // The merge orders by FIRST SIGHT. A source that joins mid-session arrives
  // holding minutes-old messages; appending them puts them BELOW the newest
  // live row, which reads as chat jumping backwards.
  const order = ['live-1', 'live-2'];
  const held = new Map(order.map((id) => [id, id]));
  const seeded = new Set<string>(['twitch::home']);

  const joining = { key: 'kick::other', backlog: ['old-1', 'old-2', 'old-3'] };
  const opening = order.length === 0;
  if (!seeded.has(joining.key)) {
    seeded.add(joining.key);
    if (!opening) for (const id of joining.backlog) if (!held.has(id)) held.set(id, id);
  }

  assert.deepEqual(order, ['live-1', 'live-2'], 'nothing was appended');
  assert.equal(held.has('old-3'), true, 'but the backlog is known, so it is not re-offered as new');
});

test('on the very first tick there is no live bottom to protect', () => {
  // Opening the feed with an empty order array must KEEP the backlog, or a
  // freshly opened combined chat would start completely blank.
  const order: string[] = [];
  const held = new Map<string, string>();
  const opening = order.length === 0;
  const backlog = ['a', 'b'];
  if (opening) for (const id of backlog) { order.push(id); held.set(id, id); }
  assert.deepEqual(order, ['a', 'b']);
});
