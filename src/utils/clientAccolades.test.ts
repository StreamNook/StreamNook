import { describe, expect, it } from 'vitest';
import {
  CLIENT_ACCOLADE_TOASTS,
  createEarner,
  restlessReached,
  storageAnnouncedStore,
  type AnnouncedStore,
} from './clientAccolades';

/** An announced store in memory, per account. */
function memoryStore(): AnnouncedStore & { sets: Map<string, Set<string>> } {
  const sets = new Map<string, Set<string>>();
  return {
    sets,
    read: (userId) => sets.get(userId) ?? new Set(),
    add: (userId, id) => sets.set(userId, new Set(sets.get(userId) ?? []).add(id)),
  };
}

/** A grant the test settles by hand. */
function pendingGrants() {
  const calls: Array<{ id: string; ok: (v: boolean) => void; fail: (e: Error) => void }> = [];
  return {
    calls,
    grant: (id: string) =>
      new Promise<boolean>((ok, reject) => {
        calls.push({ id, ok, fail: reject });
      }),
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function rig(userId: string | null = 'u1') {
  const state = { userId };
  const toasts: string[] = [];
  const grants = pendingGrants();
  const announced = memoryStore();
  const earn = createEarner({ userId: () => state.userId, grant: grants.grant, announced, toast: (t) => toasts.push(t) });
  return { state, toasts, grants, announced, earn };
}

describe('the ids Rust allows', () => {
  it('are exactly the four client-attested accolades, each with its own line', () => {
    expect(Object.keys(CLIENT_ACCOLADE_TOASTS).sort()).toEqual(['insomniac', 'perfect_corner', 'shiny_peepo', 'worth_the_wait']);
    expect(CLIENT_ACCOLADE_TOASTS.insomniac).toBe('Achievement unlocked: Restless. The Midnight Atmosphere is yours.');
    expect(CLIENT_ACCOLADE_TOASTS.perfect_corner).toBe('Achievement unlocked: Perfect Corner.');
  });
});

describe('Restless', () => {
  it('is reached from the fiftieth click on, not only at it', () => {
    expect(restlessReached(49)).toBe(false);
    expect(restlessReached(50)).toBe(true);
    expect(restlessReached(51)).toBe(true);
    expect(restlessReached(0)).toBe(false);
  });
});

describe('earning a client-attested accolade', () => {
  it('toasts and announces only after Rust says yes', async () => {
    const r = rig();
    const pending = r.earn('perfect_corner');
    await flush();
    expect(r.grants.calls).toHaveLength(1);
    expect(r.toasts).toEqual([]);
    expect(r.announced.read('u1').has('perfect_corner')).toBe(false);
    r.grants.calls[0].ok(true);
    await pending;
    expect(r.toasts).toEqual(['Achievement unlocked: Perfect Corner.']);
    expect(r.announced.read('u1').has('perfect_corner')).toBe(true);
  });

  it('joins the announced set without a toast when Rust already held the accolade', async () => {
    const r = rig();
    const pending = r.earn('shiny_peepo');
    await flush();
    r.grants.calls[0].ok(false);
    await pending;
    expect(r.toasts).toEqual([]);
    expect(r.announced.read('u1').has('shiny_peepo')).toBe(true);
    await r.earn('shiny_peepo');
    expect(r.grants.calls).toHaveLength(1);
  });

  it('a refused request announces nothing and the next moment asks again', async () => {
    const r = rig();
    const first = r.earn('worth_the_wait');
    await flush();
    r.grants.calls[0].fail(new Error('offline'));
    await first;
    expect(r.toasts).toEqual([]);
    expect(r.announced.read('u1').has('worth_the_wait')).toBe(false);
    const second = r.earn('worth_the_wait');
    await flush();
    expect(r.grants.calls).toHaveLength(2);
    r.grants.calls[1].ok(true);
    await second;
    expect(r.toasts).toEqual(['Achievement unlocked: Worth the Wait.']);
  });

  it('does not start a second request while one is running', async () => {
    const r = rig();
    const a = r.earn('insomniac');
    const b = r.earn('insomniac');
    const c = r.earn('insomniac');
    await flush();
    expect(r.grants.calls).toHaveLength(1);
    r.grants.calls[0].ok(true);
    await Promise.all([a, b, c]);
    expect(r.toasts).toHaveLength(1);
  });

  it('does not ask again for an id already announced for this account', async () => {
    const r = rig();
    const first = r.earn('insomniac');
    await flush();
    r.grants.calls[0].ok(true);
    await first;
    await r.earn('insomniac');
    expect(r.grants.calls).toHaveLength(1);
    expect(r.toasts).toHaveLength(1);
  });

  it('keeps the announced set per account: another account earns and is told on its own', async () => {
    const r = rig('u1');
    const first = r.earn('insomniac');
    await flush();
    r.grants.calls[0].ok(true);
    await first;
    r.state.userId = 'u2';
    const second = r.earn('insomniac');
    await flush();
    expect(r.grants.calls).toHaveLength(2);
    r.grants.calls[1].ok(true);
    await second;
    expect(r.toasts).toHaveLength(2);
    expect([...r.announced.sets.keys()].sort()).toEqual(['u1', 'u2']);
  });

  it('says nothing to an account that took over while Rust was answering, but remembers for the one it was for', async () => {
    const r = rig('u1');
    const pending = r.earn('shiny_peepo');
    await flush();
    r.state.userId = 'u2';
    r.grants.calls[0].ok(true);
    await pending;
    expect(r.toasts).toEqual([]);
    expect(r.announced.read('u1').has('shiny_peepo')).toBe(true);
    expect(r.announced.read('u2').has('shiny_peepo')).toBe(false);
  });

  it('does nothing signed out, and nothing for an id off the allowlist', async () => {
    const out = rig(null);
    await out.earn('insomniac');
    expect(out.grants.calls).toHaveLength(0);
    const r = rig();
    await r.earn('chatterbox');
    await r.earn('hoarder');
    expect(r.grants.calls).toHaveLength(0);
  });
});

describe('the announced set on this device', () => {
  const fakeStorage = () => {
    const data = new Map<string, string>();
    return {
      data,
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
      removeItem: (k: string) => void data.delete(k),
    };
  };

  it('is kept per account', () => {
    const storage = fakeStorage();
    const store = storageAnnouncedStore(storage);
    store.add('u1', 'insomniac');
    store.add('u1', 'shiny_peepo');
    store.add('u2', 'insomniac');
    expect([...store.read('u1')].sort()).toEqual(['insomniac', 'shiny_peepo']);
    expect([...store.read('u2')]).toEqual(['insomniac']);
    expect([...store.read('u3')]).toEqual([]);
  });

  it('reads a broken value as empty and survives a storage that throws', () => {
    const broken = { getItem: () => '{not json', setItem: () => undefined, removeItem: () => undefined };
    expect([...storageAnnouncedStore(broken).read('u1')]).toEqual([]);
    const throwing = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
      removeItem: () => {
        throw new Error('blocked');
      },
    };
    const store = storageAnnouncedStore(throwing);
    expect([...store.read('u1')]).toEqual([]);
    expect(() => store.add('u1', 'insomniac')).not.toThrow();
    expect([...storageAnnouncedStore(null).read('u1')]).toEqual([]);
  });

  it('moves the old device-wide offline set into the first account that reads, once', () => {
    const storage = fakeStorage();
    storage.data.set('sn:offline-accolades', JSON.stringify(['perfect_corner', 'shiny_peepo']));
    const store = storageAnnouncedStore(storage);
    expect([...store.read('u1')].sort()).toEqual(['perfect_corner', 'shiny_peepo']);
    // The old key is gone and the ids are now u1's own.
    expect(storage.data.has('sn:offline-accolades')).toBe(false);
    expect(JSON.parse(storage.data.get('sn:client-accolades:u1') ?? '[]').sort()).toEqual(['perfect_corner', 'shiny_peepo']);
    // A second account starts with nothing.
    expect([...store.read('u2')]).toEqual([]);
  });

  it('merges the old set with what the account already had, and drops a broken old key', () => {
    const storage = fakeStorage();
    storage.data.set('sn:client-accolades:u1', JSON.stringify(['insomniac']));
    storage.data.set('sn:offline-accolades', JSON.stringify(['worth_the_wait']));
    expect([...storageAnnouncedStore(storage).read('u1')].sort()).toEqual(['insomniac', 'worth_the_wait']);
    const broken = fakeStorage();
    broken.data.set('sn:offline-accolades', '{not json');
    expect([...storageAnnouncedStore(broken).read('u1')]).toEqual([]);
    expect(broken.data.has('sn:offline-accolades')).toBe(false);
  });

  it('keeps the old key when it cannot write the account set', () => {
    const data = new Map<string, string>([['sn:offline-accolades', JSON.stringify(['shiny_peepo'])]]);
    const storage = {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: () => {
        throw new Error('full');
      },
      removeItem: (k: string) => void data.delete(k),
    };
    storageAnnouncedStore(storage).read('u1');
    expect(data.has('sn:offline-accolades')).toBe(true);
  });
});
