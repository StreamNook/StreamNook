// Performance mode takes the backdrop blur away without touching the viewer's
// Blur switch: the page reads blur off AND draws the Glassiness 0% look (solid
// surfaces), a theme or glass save while it is active keeps both, and the
// viewer's own choice (the 0.45 blur-off cap when their switch is off) comes
// back after.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: () => Promise.resolve({}) }));

import { applyGlassBlur, applyGlassStrength, setPerformanceFlat } from './index';

const vars = new Map<string, string>();
const attrs = new Map<string, string>();
beforeEach(() => {
  vars.clear();
  attrs.clear();
  vi.stubGlobal('document', {
    documentElement: {
      dataset: {} as Record<string, string>,
      style: {
        setProperty: (name: string, value: string) => vars.set(name, value),
        getPropertyValue: (name: string) => vars.get(name) ?? '',
      },
      setAttribute: (name: string, value: string) => attrs.set(name, value),
      removeAttribute: (name: string) => attrs.delete(name),
    },
    body: { setAttribute: () => {} },
  });
  vi.stubGlobal('window', { dispatchEvent: () => true });
  setPerformanceFlat(false);
  applyGlassBlur(true);
  applyGlassStrength(100);
});
afterEach(() => {
  setPerformanceFlat(false);
  applyGlassBlur(true);
  vi.unstubAllGlobals();
});

describe('blur with Performance mode', () => {
  it('flat turns blur off and draws solid surfaces', () => {
    expect(attrs.get('data-blur')).toBe('on');
    expect(vars.get('--glass-strength')).toBe('1');
    setPerformanceFlat(true);
    expect(attrs.get('data-blur')).toBe('off');
    expect(vars.get('--glass-strength')).toBe('0');
  });

  it('a save that re-applies the Blur switch while flat keeps blur off and solid surfaces', () => {
    setPerformanceFlat(true);
    applyGlassBlur(true);
    applyGlassStrength(100);
    expect(attrs.get('data-blur')).toBe('off');
    expect(vars.get('--glass-strength')).toBe('0');
  });

  it('flat goes solid even when the Blur switch was already off', () => {
    applyGlassBlur(false);
    expect(vars.get('--glass-strength')).toBe('0.45');
    setPerformanceFlat(true);
    expect(attrs.get('data-blur')).toBe('off');
    expect(vars.get('--glass-strength')).toBe('0');
    setPerformanceFlat(false);
    expect(vars.get('--glass-strength')).toBe('0.45');
  });

  it("leaving flat gives the viewer's choice back", () => {
    setPerformanceFlat(true);
    setPerformanceFlat(false);
    expect(attrs.get('data-blur')).toBe('on');
    expect(vars.get('--glass-strength')).toBe('1');
    applyGlassBlur(false);
    setPerformanceFlat(true);
    setPerformanceFlat(false);
    expect(attrs.get('data-blur')).toBe('off');
    expect(vars.get('--glass-strength')).toBe('0.45');
  });
});
