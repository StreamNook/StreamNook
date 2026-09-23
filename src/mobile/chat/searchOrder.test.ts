import { describe, it, expect } from 'vitest';
import { orderSearchResults } from './searchOrder';
import type { TwitchStream } from '../../types';

const row = (login: string, isLive: boolean, viewers = 0): TwitchStream =>
  ({ id: login, user_id: login, user_login: login, user_name: login, viewer_count: viewers, is_live: isLive }) as TwitchStream;

describe('orderSearchResults', () => {
  it('puts a live channel ahead of offline lookalikes the platform ranked higher', () => {
    const out = orderSearchResults([row('mutex_backup1', false), row('mutexcs', false), row('mutex', true, 227)]);
    expect(out.map((r) => r.user_login)).toEqual(['mutex', 'mutex_backup1', 'mutexcs']);
  });

  it('orders live channels by viewers and keeps offline ones in platform order', () => {
    const out = orderSearchResults([
      row('off-b', false),
      row('small', true, 5),
      row('off-a', false),
      row('big', true, 900),
    ]);
    expect(out.map((r) => r.user_login)).toEqual(['big', 'small', 'off-b', 'off-a']);
  });
});
