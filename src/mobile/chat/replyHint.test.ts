import { beforeEach, describe, expect, it, vi } from 'vitest';

/** A device's storage for the hint, fresh per test. */
function storage(): Storage {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, String(v)),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
    key: (i: number) => [...m.keys()][i] ?? null,
    get length() {
      return m.size;
    },
  };
}

describe('the reply hint', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.stubGlobal('localStorage', storage());
  });

  it('shows on five pauses and goes on the sixth', async () => {
    const hint = await import('./replyHint');
    const learned = () => localStorage.getItem('sn-swipe-reply-learned') === '1';
    for (let i = 0; i < 5; i++) hint.replyHintOnPause();
    expect(learned()).toBe(false);
    hint.replyHintOnPause();
    expect(learned()).toBe(true);
  });

  it('goes for good after a reply swipe', async () => {
    const hint = await import('./replyHint');
    hint.learnReplyHint();
    expect(localStorage.getItem('sn-swipe-reply-learned')).toBe('1');
    // A learned hint counts no more pauses.
    hint.replyHintOnPause();
    expect(localStorage.getItem('sn-swipe-reply-hint-shown')).toBeNull();
  });
});
