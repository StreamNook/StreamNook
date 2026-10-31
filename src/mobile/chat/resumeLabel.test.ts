import { describe, expect, it } from 'vitest';
import { resumeCountText, resumeLabel } from './resumeLabel';

describe('the paused chat resume button', () => {
  it('says where it goes before anything new arrives', () => {
    expect(resumeLabel(0)).toBe('Scroll to the latest messages');
  });

  it('reads the full count, one message or many', () => {
    expect(resumeLabel(1)).toBe('1 new message, scrolls to the latest messages');
    expect(resumeLabel(43)).toBe('43 new messages, scrolls to the latest messages');
    // The circle shows at most "99+"; the label keeps the real number.
    expect(resumeLabel(1200)).toBe('1200 new messages, scrolls to the latest messages');
  });

  it('counts as +N, never more than three characters in the circle', () => {
    expect(resumeCountText(0)).toBe('');
    expect(resumeCountText(1)).toBe('+1');
    expect(resumeCountText(42)).toBe('+42');
    expect(resumeCountText(99)).toBe('+99');
    expect(resumeCountText(100)).toBe('99+');
    for (const n of [1, 9, 42, 99, 100, 5000]) expect(resumeCountText(n).length).toBeLessThanOrEqual(3);
  });
});
