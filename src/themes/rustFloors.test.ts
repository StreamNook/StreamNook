// applyTheme paints the palette it holds, then takes Rust's contrast floors
// (`lift_theme_palette`) for muted and secondary text and the accent on cards.
// Custom themes, OLED in a picked accent and the editor's preview only get
// those from Rust, and an accent dragged in the editor fires many asks, so
// only the newest answer may paint.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  invoke: vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => h.invoke(cmd, args) }));

import { applyTheme, getOledTheme, getThemeById, type Theme } from './index';

// The suite runs in node: just enough of a document for applyTheme to paint.
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

const deferred = () => {
  let resolve!: (v: unknown) => void;
  const promise = new Promise<unknown>((r) => {
    resolve = r;
  });
  return { promise, resolve };
};

// Lets the queued then-chains run.
const settle = () => new Promise<void>((r) => setTimeout(r, 0));

const withAccent = (base: Theme, id: string, accent: string): Theme => ({
  ...base,
  id,
  palette: { ...base.palette, accent, accentOnSurface: undefined },
});

describe('applyTheme takes Rust floors', () => {
  it('asks lift_theme_palette with the palette as painted and paints its answer', async () => {
    h.invoke.mockResolvedValue({
      accentOnSurface: '#5c7cff',
      textMuted: 'rgba(200, 200, 200, 0.62)',
      textSecondary: '#c8c8d0',
    });
    const theme = withAccent(getThemeById('winters-glass')!, 'custom-a', '#2040ff');
    applyTheme(theme);

    // Painted at once, before Rust answers: the accent stands in.
    expect(vars.get('--color-accent-on-surface')).toBe('#2040ff');
    expect(vars.get('--color-text-muted')).toBe(theme.palette.textMuted);

    await settle();
    expect(h.invoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = h.invoke.mock.calls[0];
    expect(cmd).toBe('lift_theme_palette');
    const sent = (args as { palette: Record<string, unknown> }).palette;
    expect(sent.accent).toBe('#2040ff');
    expect(sent.textMuted).toBe(theme.palette.textMuted);
    expect(vars.get('--color-accent-on-surface')).toBe('#5c7cff');
    expect(vars.get('--color-text-muted')).toBe('rgba(200, 200, 200, 0.62)');
    expect(vars.get('--color-text-secondary')).toBe('#c8c8d0');
  });

  it('drops a stale answer: only the newest ask paints', async () => {
    const first = deferred();
    const second = deferred();
    h.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const oled = getOledTheme('#101010');
    applyTheme(oled);
    applyTheme(getOledTheme('#202020'));
    await settle();

    second.resolve({ accentOnSurface: '#7a7a7a' });
    await settle();
    first.resolve({ accentOnSurface: '#555555' });
    await settle();
    expect(vars.get('--color-accent-on-surface')).toBe('#7a7a7a');
  });

  it('a repaint of the same theme keeps the floored colors until the new answer', async () => {
    h.invoke.mockResolvedValueOnce({ accentOnSurface: '#7a7a7a' });
    applyTheme(getOledTheme('#101010'));
    await settle();
    expect(vars.get('--color-accent-on-surface')).toBe('#7a7a7a');

    const next = deferred();
    h.invoke.mockReturnValueOnce(next.promise);
    applyTheme(getOledTheme('#181818'));
    // No flash back to the raw accent while the ask is out.
    expect(vars.get('--color-accent-on-surface')).toBe('#7a7a7a');
    next.resolve({ accentOnSurface: '#808080' });
    await settle();
    expect(vars.get('--color-accent-on-surface')).toBe('#808080');
  });

  it('outside Tauri a failed ask leaves the painted values', async () => {
    h.invoke.mockRejectedValue(new Error('no bridge'));
    const theme = withAccent(getThemeById('winters-glass')!, 'custom-b', '#2040ff');
    applyTheme(theme);
    await settle();
    expect(vars.get('--color-accent-on-surface')).toBe('#2040ff');
    expect(vars.get('--color-text-muted')).toBe(theme.palette.textMuted);
    expect(vars.get('--color-text-secondary')).toBe(theme.palette.textSecondary);
  });
});
