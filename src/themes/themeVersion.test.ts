// What a page measured from the painted theme (a song page fitting its cover's
// colors to the muted text) is measured again whenever those colors may have
// changed: a theme applied, Rust's floors landing after it, the glass setters.
// A repaint that writes the same values asks for nothing.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  invoke: vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => h.invoke(cmd, args) }));

import { applyGlassBlur, applyGlassStrength, applyTheme, getThemeById, subscribeTheme, themeVersion } from './index';

const vars = new Map<string, string>();
beforeEach(() => {
  vars.clear();
  h.invoke.mockReset();
  vi.stubGlobal('document', {
    documentElement: {
      dataset: {} as Record<string, string>,
      style: {
        setProperty: (name: string, value: string) => vars.set(name, value),
        getPropertyValue: (name: string) => vars.get(name) ?? '',
      },
      setAttribute: () => {},
      removeAttribute: () => {},
    },
    body: { setAttribute: () => {} },
  });
  vi.stubGlobal('window', { dispatchEvent: () => true });
});
afterEach(() => {
  vi.unstubAllGlobals();
});

const settle = () => new Promise<void>((r) => setTimeout(r, 0));

describe('the theme version', () => {
  it('moves when a theme is applied, and tells its watchers', () => {
    h.invoke.mockResolvedValue({});
    const seen: number[] = [];
    const stop = subscribeTheme(() => seen.push(themeVersion()));
    const before = themeVersion();
    applyTheme(getThemeById('winters-glass')!);
    expect(themeVersion()).toBe(before + 1);
    expect(seen).toEqual([before + 1]);
    stop();
    applyTheme(getThemeById('dracula')!);
    expect(seen).toEqual([before + 1]);
  });

  it("moves again when Rust's floors land with a changed value, not with the same ones", async () => {
    const theme = getThemeById('nord')!;
    h.invoke.mockResolvedValue({ textMuted: 'rgba(1, 2, 3, 0.9)' });
    applyTheme(theme);
    const painted = themeVersion();
    await settle();
    expect(vars.get('--color-text-muted')).toBe('rgba(1, 2, 3, 0.9)');
    expect(themeVersion()).toBe(painted + 1);

    // The same theme again keeps Rust's values on screen, and the same answer
    // writes nothing new.
    applyTheme(theme);
    const repainted = themeVersion();
    await settle();
    expect(themeVersion()).toBe(repainted);
  });

  it('moves with the glass setters', () => {
    h.invoke.mockResolvedValue({});
    applyTheme(getThemeById('winters-glass')!);
    const v = themeVersion();
    applyGlassStrength(40);
    expect(themeVersion()).toBeGreaterThan(v);
    const w = themeVersion();
    applyGlassBlur(false);
    expect(themeVersion()).toBeGreaterThan(w);
    const x = themeVersion();
    applyGlassBlur(false);
    expect(themeVersion()).toBeGreaterThan(x);
    applyGlassBlur(true);
  });
});
