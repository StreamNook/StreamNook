// Cosmetics lookups must batch ACROSS event-loop tasks, not just within one.
//
// Live chat delivers one message per task, so the original end-of-tick microtask
// drain put exactly one chatter in every batch and paid a full 7TV round-trip
// for each of them. The diag line read "resolved 1/1" hundreds of times in a row
// on a large channel and never once read 5/5.
//
// The per-user query must also stay SLIM. It used to carry a full definition for
// every cosmetic a chatter owned, to render the one they wore, and that payload
// is what pinned the chunk size at 5 against 7TV's ~400 complexity ceiling.
import { describe, expect, it, vi, beforeEach } from 'vitest';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  convertFileSrc: (path: string) => path,
}));

import { getUserCosmetics, clearUserCache } from './seventvService';

/** Yield to the macrotask queue, the way one WebSocket frame follows another. */
const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));

const gqlQueries = () =>
  invokeMock.mock.calls
    .filter((call) => call[0] === 'seventv_graphql')
    .map((call) => (call[1] as { query: string }).query);

// The shared-definitions catalog rides the same transport, so per-user
// assertions must exclude it rather than counting every GraphQL call.
const userQueries = () => gqlQueries().filter((q) => q.includes('userByConnection'));

describe('7TV cosmetics batching', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ data: {} });
    clearUserCache();
  });

  it('collapses chatters queued in separate tasks into one query', async () => {
    const first = getUserCosmetics('111');
    await nextTask();
    const second = getUserCosmetics('222');
    await Promise.all([first, second]);

    const queries = userQueries();
    expect(queries).toHaveLength(1);
    expect(queries[0]).toContain('u_111');
    expect(queries[0]).toContain('u_222');
  });

  it('asks only what a chatter is WEARING, never their whole inventory', async () => {
    await getUserCosmetics('111');

    const [query] = userQueries();
    expect(query).toBeDefined();
    // The point of the split: definitions come from the catalog, so the per-user
    // query carries active ids only. Putting `inventory` back here silently costs
    // ~6x the batch size, because complexity per user jumps back to ~71.
    expect(query).not.toContain('inventory');
    expect(query).toContain('activePaint');
    expect(query).toContain('activeBadge');
  });

  it('fits a burst well past the old 5-user ceiling into one query', async () => {
    const ids = Array.from({ length: 25 }, (_, i) => `${1000 + i}`);
    await Promise.all(ids.map((id) => getUserCosmetics(id)));

    const queries = userQueries();
    // 25 users cost five round-trips under the inventory-carrying query. They
    // now fit in one.
    expect(queries).toHaveLength(1);
    expect(queries[0].match(/userByConnection/g) ?? []).toHaveLength(25);
  });

  it('still chunks once the burst exceeds the measured complexity ceiling', async () => {
    const ids = Array.from({ length: 65 }, (_, i) => `${2000 + i}`);
    await Promise.all(ids.map((id) => getUserCosmetics(id)));

    const queries = userQueries();
    // Pulls the opposite way from the test above on purpose: "send everything in
    // one query" would satisfy that one and get the whole batch rejected as too
    // complex, stranding every user in it without cosmetics.
    expect(queries.length).toBeGreaterThan(1);
    for (const query of queries) {
      const users = query.match(/userByConnection/g) ?? [];
      expect(users.length).toBeLessThanOrEqual(30);
    }
    expect(queries.join(' ')).toContain('u_2064');
  });
});
