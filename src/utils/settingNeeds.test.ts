import { describe, expect, it, vi } from 'vitest';
import { setSettingNeed, settingNeedMet, settingNeedsVersion, subscribeSettingNeeds } from './settingNeeds';

describe('setting needs', () => {
  it('treats a need nobody reported as unmet', () => {
    expect(settingNeedMet('never_reported')).toBe(false);
  });

  it('records a need and moves the version only when the answer changes', () => {
    const before = settingNeedsVersion();
    setSettingNeed('records_a_need', true);
    expect(settingNeedMet('records_a_need')).toBe(true);
    expect(settingNeedsVersion()).toBe(before + 1);

    setSettingNeed('records_a_need', true);
    expect(settingNeedsVersion()).toBe(before + 1);

    setSettingNeed('records_a_need', false);
    expect(settingNeedMet('records_a_need')).toBe(false);
    expect(settingNeedsVersion()).toBe(before + 2);
  });

  it('does not count a first report of unmet as a change', () => {
    const before = settingNeedsVersion();
    setSettingNeed('first_unmet', false);
    expect(settingNeedsVersion()).toBe(before);
  });

  it('tells subscribers on a change, not on a repeat, and not after they leave', () => {
    const seen = vi.fn();
    const off = subscribeSettingNeeds(seen);
    setSettingNeed('tells_subscribers', true);
    setSettingNeed('tells_subscribers', true);
    expect(seen).toHaveBeenCalledTimes(1);
    off();
    setSettingNeed('tells_subscribers', false);
    expect(seen).toHaveBeenCalledTimes(1);
  });
});
