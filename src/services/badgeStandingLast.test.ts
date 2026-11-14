import { describe, expect, it, vi } from 'vitest';

// Each call returns a promise the test settles by hand, so answers can land out of order.
const pending: Array<(value: unknown) => void> = [];
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => new Promise((resolve) => pending.push(resolve))),
}));

import { getBadgeStanding, getLastStanding, type BadgeStanding } from './badgeStanding';

const standing = (tag: string) => ({ owned: [tag] }) as unknown as BadgeStanding;

describe('getLastStanding', () => {
  it('keeps the newest request, not the answer that landed last', async () => {
    const older = getBadgeStanding();
    const newer = getBadgeStanding(true);
    pending[1](standing('newer'));
    await newer;
    pending[0](standing('older'));
    await older;
    expect(getLastStanding()?.owned).toEqual(['newer']);
  });
});
