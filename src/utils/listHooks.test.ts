import { beforeEach, describe, expect, it, vi } from 'vitest';

// "Notify me about" came up empty on 8.9.1 (zoox, Brandon): the list landed
// after the hook's first render and before its effect, and the effect, seeing
// it already cached, returned without handing it to the page. React is
// stood in for here so that exact order can be played.

const effects: Array<() => void | (() => void)> = [];
const setState = vi.fn();
vi.mock('react', () => ({
  useState: (init: unknown) => [init, setState],
  useEffect: (fn: () => void | (() => void)) => {
    effects.push(fn);
  },
}));
const ALERTS = [{ id: 'live', setting: 'live_notifications.show_live_notifications' }];
const NOTICES = [{ id: 'copied', setting: 'live_notifications.notice_copied' }];
vi.mock('./invokeList', () => ({
  invokeList: vi.fn(async (command: string) => (command === 'get_alert_types' ? ALERTS : NOTICES)),
}));

import { loadAlertTypes, useAlertTypes } from '../mobile/alertTypes';
import { loadNoticeTypes, useNoticeTypes } from './notices';

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  effects.length = 0;
  setState.mockReset();
});

describe('a list that lands between the first render and the effect', () => {
  it('still reaches Notify me about', async () => {
    expect(useAlertTypes()).toBeNull();
    await loadAlertTypes();
    for (const run of effects) run();
    await flush();
    expect(setState).toHaveBeenCalledWith(ALERTS);
  });

  it('still reaches Small notices', async () => {
    expect(useNoticeTypes()).toBeNull();
    await loadNoticeTypes();
    for (const run of effects) run();
    await flush();
    expect(setState).toHaveBeenCalledWith(NOTICES);
  });
});
