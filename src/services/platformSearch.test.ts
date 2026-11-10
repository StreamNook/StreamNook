import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ Channel: class {}, invoke: vi.fn() }));

import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
import { mergeSearchBatches, type SearchBatch } from './platformSearch';

const row = (user_login: string, viewer_count: number, is_live = true) =>
  ({ user_login, viewer_count, is_live }) as TwitchStream;

const batch = (provider: ProviderId, rows: TwitchStream[], ranks: number[]): SearchBatch => ({
  provider,
  streams: rows,
  ranks,
  error: null,
});

describe('mergeSearchBatches', () => {
  it('puts the channel itself first whichever platform it is on', () => {
    const found = new Map<ProviderId, SearchBatch>([
      ['twitch', batch('twitch', [row('ishowspeedclips', 40), row('xqc', 90_000)], [1, 5])],
      ['youtube', batch('youtube', [row('IShowSpeed', 120_000)], [0])],
    ]);
    expect(mergeSearchBatches(['twitch', 'youtube', 'kick'], found).map((r) => r.user_login)).toEqual([
      'IShowSpeed',
      'ishowspeedclips',
      'xqc',
    ]);
  });

  it('within one match strength, live and then most watched lead, then platform order', () => {
    const found = new Map<ProviderId, SearchBatch>([
      ['twitch', batch('twitch', [row('a_offline', 0, false), row('b_small', 5)], [2, 2])],
      ['youtube', batch('youtube', [row('c_big', 500), row('d_small', 5)], [2, 2])],
    ]);
    expect(mergeSearchBatches(['twitch', 'youtube'], found).map((r) => r.user_login)).toEqual([
      'c_big',
      'b_small',
      'd_small',
      'a_offline',
    ]);
  });

  it('a platform that has not answered yet adds nothing', () => {
    const found = new Map<ProviderId, SearchBatch>([['twitch', batch('twitch', [row('a', 1)], [0])]]);
    expect(mergeSearchBatches(['twitch', 'youtube'], found)).toHaveLength(1);
  });
});
