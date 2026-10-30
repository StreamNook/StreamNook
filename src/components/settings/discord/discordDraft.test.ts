import { describe, expect, it, vi } from 'vitest';

vi.mock('../../../utils/settingsFields', () => ({
  setSettingsFields: vi.fn(async () => {}),
  fieldsIn: (group: string, changes: Record<string, unknown>) =>
    Object.fromEntries(Object.entries(changes).map(([k, v]) => [`${group}.${k}`, v ?? null])),
}));
vi.mock('../../../utils/settingsBroadcast', () => ({ SENDER_ID: 'page-test' }));

import type { DiscordPresenceSettings } from '../../../types';
import type { WriteOrder } from '../../../utils/settingsFields';
import { createCommitter, draftOf, reconcile, type CommitterDeps } from './discordDraft';

const saved = {
  large_image: 'category',
  watching_top: 'Watching {streamer}',
} as unknown as DiscordPresenceSettings;

describe('draft', () => {
  it('shows pending fields over the saved style', () => {
    expect(draftOf(saved, { large_image: 'streamer' }).large_image).toBe('streamer');
  });

  it('keeps a field pending until the saved style shows it', () => {
    const pending = { watching_top: 'Watching {streamer} now' };
    // The write answered, but the re-read has not landed: still pending, no flash back.
    expect(reconcile(saved, pending)).toEqual(pending);
    const landed = { ...saved, watching_top: 'Watching {streamer} now' };
    expect(reconcile(landed, pending)).toEqual({});
  });

  it('keeps a keystroke made between the write and the re-read', () => {
    const typedMore = { watching_top: 'Watching {streamer} now!' };
    const landed = { ...saved, watching_top: 'Watching {streamer} now' };
    expect(reconcile(landed, typedMore)).toEqual(typedMore);
  });

  it('a reset during a pending commit stays pending until saved', () => {
    const reset = { large_image: 'category' } as const;
    const savedStreamer = { ...saved, large_image: 'streamer' } as DiscordPresenceSettings;
    expect(reconcile(savedStreamer, reset)).toEqual(reset);
    expect(reconcile(saved, reset)).toEqual({});
  });
});

/** A committer whose writes each wait to be answered, in any order. */
function harness() {
  const writes: Array<{
    changes: Record<string, unknown>;
    order: WriteOrder;
    resolve: () => void;
    reject: () => void;
  }> = [];
  const retries: Array<() => void> = [];
  const deps: CommitterDeps = {
    stream: 'discord_presence:page-test',
    write: (changes, order) =>
      new Promise<void>((resolve, reject) => {
        writes.push({ changes, order, resolve, reject: () => reject(new Error('down')) });
      }),
    later: (fn) => {
      retries.push(fn);
      return () => {
        const i = retries.indexOf(fn);
        if (i >= 0) retries.splice(i, 1);
      };
    },
  };
  return { writes, retries, deps };
}

const settle = () => new Promise((r) => setTimeout(r, 0));
const top = (w: { changes: Record<string, unknown> }) => w.changes['discord_presence.watching_top'];

describe('committer', () => {
  it('writes one numbered batch at a time and merges what came meanwhile', async () => {
    const h = harness();
    const c = createCommitter(h.deps);
    c.queue({ watching_top: 'a' });
    c.queue({ watching_top: 'ab' });
    c.queue({ large_image: 'streamer' });
    expect(h.writes.map((w) => w.changes)).toEqual([{ 'discord_presence.watching_top': 'a' }]);
    expect(c.status()).toBe('saving');
    h.writes[0].resolve();
    await settle();
    expect(h.writes[1].changes).toEqual({ 'discord_presence.watching_top': 'ab', 'discord_presence.large_image': 'streamer' });
    expect(h.writes.map((w) => w.order.n)).toEqual([1, 2]);
    expect(h.writes[0].order.stream).toBe('discord_presence:page-test');
    h.writes[1].resolve();
    await settle();
    expect(c.status()).toBe('idle');
  });

  it('retries a failed write after 2 s, newer edits winning; flush skips the wait', async () => {
    const h = harness();
    const c = createCommitter(h.deps);
    c.queue({ watching_top: 'a' });
    h.writes[0].reject();
    await settle();
    expect(c.status()).toBe('retrying');
    expect(h.retries).toHaveLength(1);
    c.queue({ watching_top: 'b' });
    expect(h.writes).toHaveLength(1);
    c.flush();
    expect(h.retries, 'flush cancelled the retry wait').toHaveLength(0);
    expect(top(h.writes[1])).toBe('b');
  });

  it('a flush while a write is in flight sends everything at once, numbered after it', () => {
    const h = harness();
    const c = createCommitter(h.deps);
    c.queue({ watching_top: 'ab' });
    c.queue({ watching_top: 'abc' });
    c.flush();
    expect(h.writes.map((w) => [top(w), w.order.n])).toEqual([
      ['ab', 1],
      ['abc', 2],
    ]);
  });

  it('an urgent write that fails after the flight finished is retried', async () => {
    const h = harness();
    const c = createCommitter(h.deps);
    c.queue({ watching_top: 'ab' });
    c.queue({ watching_top: 'abc' });
    c.flush();
    h.writes[0].resolve();
    await settle();
    h.writes[1].reject();
    await settle();
    expect(c.status()).toBe('retrying');
    h.retries[0]();
    expect(top(h.writes[2])).toBe('abc');
  });

  it('a write after an unanswered urgent write carries its fields', async () => {
    const h = harness();
    const c = createCommitter(h.deps);
    c.queue({ watching_top: 'ab' });
    c.queue({ watching_top: 'abc' });
    c.flush();
    h.writes[0].resolve();
    await settle();
    c.queue({ large_image: 'streamer' });
    expect(h.writes[2].changes).toEqual({
      'discord_presence.watching_top': 'abc',
      'discord_presence.large_image': 'streamer',
    });
    expect(h.writes[2].order.n).toBe(3);
  });
});
