import { describe, expect, it } from 'vitest';
import { deriveShape } from './useWindowShape';

describe('deriveShape', () => {
  it('has no vertical split without a hinge', () => {
    const s = deriveShape({ w: 412, h: 915 }, null);
    expect(s.splitY).toBeNull();
    expect(s.sizeClass).toBe('compact');
    expect(s.twoPane).toBe(false);
  });

  it('splits on a half-open horizontal hinge (tabletop)', () => {
    const s = deriveShape(
      { w: 412, h: 915 },
      { vertical: false, posture: 'half', x: 0, width: 412, y: 450, height: 14 },
    );
    expect(s.splitY).toBe(457);
    expect(s.splitX).toBe(Math.round(412 * 0.62));
  });

  it('ignores a flat horizontal hinge', () => {
    const s = deriveShape(
      { w: 412, h: 915 },
      { vertical: false, posture: 'flat', x: 0, width: 412, y: 450, height: 14 },
    );
    expect(s.splitY).toBeNull();
  });

  it('snaps a vertical hinge to splitX and never sets splitY', () => {
    const s = deriveShape(
      { w: 840, h: 757 },
      { vertical: true, posture: 'flat', x: 415, width: 10, y: 0, height: 757 },
    );
    expect(s.splitX).toBe(420);
    expect(s.splitY).toBeNull();
    expect(s.sizeClass).toBe('expanded');
    expect(s.twoPane).toBe(true);
  });

  it('gives a medium landscape window two panes and a medium portrait one none', () => {
    expect(deriveShape({ w: 700, h: 500 }, null).twoPane).toBe(true);
    expect(deriveShape({ w: 700, h: 900 }, null).twoPane).toBe(false);
  });
});
