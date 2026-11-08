import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  invoke: vi.fn((..._args: unknown[]) => Promise.resolve()),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('./logger', () => ({ Logger: { debug: vi.fn() } }));

import { claimWhisper, clearWhisperClaim } from './whisperClaim';

describe('whisper conversation claims', () => {
  beforeEach(() => h.invoke.mockClear());

  it('numbers every call of one page load so Rust can order them', () => {
    clearWhisperClaim();
    claimWhisper('1000000000');
    claimWhisper(null);
    const calls = h.invoke.mock.calls;
    expect(calls.every((c) => c[0] === 'whisper_set_active')).toBe(true);
    const args = calls.map((c) => c[1] as { key: string | null; pageLoad: string; seq: number });
    expect(args.map((a) => a.key)).toEqual([null, '1000000000', null]);
    expect(new Set(args.map((a) => a.pageLoad)).size).toBe(1);
    expect(args[1].seq).toBeGreaterThan(args[0].seq);
    expect(args[2].seq).toBeGreaterThan(args[1].seq);
  });
});
