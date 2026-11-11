import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../stores/AppStore', () => ({ useAppStore: Object.assign(() => undefined, { getState: () => ({ settings: {} }), setState: vi.fn() }) }));

import { WATCHABLE_PROVIDERS } from '../types/providers';
import { readSuggestOff, suggestOn, toggledOff } from './suggestPlatforms';

describe('suggest platforms', () => {
  it('reads only known platforms, and never every one of them as off', () => {
    expect(readSuggestOff(['youtube', 'rumble', 'kick'])).toEqual(WATCHABLE_PROVIDERS.filter((p) => p === 'youtube' || p === 'kick'));
    expect(readSuggestOff('youtube')).toEqual([]);
    expect(readSuggestOff([...WATCHABLE_PROVIDERS])).toEqual([]);
  });

  it('a press turns a platform off, and back on', () => {
    const off = toggledOff([], 'youtube');
    expect(off).toEqual(['youtube']);
    expect(suggestOn(off)).not.toContain('youtube');
    expect(toggledOff(off, 'youtube')).toEqual([]);
  });

  it('the last platform on stays on', () => {
    const onlyTwitch = WATCHABLE_PROVIDERS.filter((p) => p !== 'twitch');
    expect(suggestOn(onlyTwitch)).toEqual(['twitch']);
    expect(toggledOff(onlyTwitch, 'twitch')).toEqual(onlyTwitch);
  });
});
