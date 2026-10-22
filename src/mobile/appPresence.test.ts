import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { applyPresence, isMainParked } from './appPresence';

describe('appPresence', () => {
  it('follows main_parked', () => {
    applyPresence({ main_parked: true });
    expect(isMainParked()).toBe(true);
    applyPresence({ main_parked: false });
    expect(isMainParked()).toBe(false);
  });
});
