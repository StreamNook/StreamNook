import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));
vi.mock('../utils/logger', () => ({ Logger: { info: vi.fn(), warn: vi.fn(), debug: vi.fn() } }));

import { nextCap } from './videoCap';

describe('videoCap', () => {
  it('sends a cap only when the rung changes', () => {
    expect(nextCap(1440, 0)).toBe(1440);
    expect(nextCap(1440, 1440)).toBeNull();
    expect(nextCap(1300, 1440)).toBe(1080);
    expect(nextCap(0, 0)).toBeNull();
  });
});
