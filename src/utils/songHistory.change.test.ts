// An open song's detail reads its song again when it was filed again or its
// cover just got its colors (the wash then fades in), and goes back to the
// list when the song is gone.
import { describe, expect, it, vi } from 'vitest';

vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { detailChange } from './songHistory';

describe("what a change means for a song's detail", () => {
  it('reads the song again when its cover got its colors', () => {
    expect(detailChange({ kind: 'tinted', keys: ['b', 'a'] }, 'a')).toBe('reload');
    expect(detailChange({ kind: 'tinted', keys: ['b'] }, 'a')).toBeNull();
  });

  it('reads it again when it was filed again', () => {
    expect(detailChange({ kind: 'filed', key: 'a' }, 'a')).toBe('reload');
    expect(detailChange({ kind: 'filed', key: 'b' }, 'a')).toBeNull();
  });

  it('is gone when removed or cleared', () => {
    expect(detailChange({ kind: 'removed', key: 'a' }, 'a')).toBe('gone');
    expect(detailChange({ kind: 'removed', key: 'b' }, 'a')).toBeNull();
    expect(detailChange({ kind: 'cleared' }, 'a')).toBe('gone');
  });
});
