import { afterEach, describe, expect, it, vi } from 'vitest';

// platform.ts reads the window label when it loads, so every case loads a fresh
// copy of the module under a stubbed window.
async function load(label: string | undefined) {
  vi.resetModules();
  vi.stubGlobal(
    'window',
    label === undefined ? {} : { __TAURI_INTERNALS__: { metadata: { currentWindow: { label } } } },
  );
  return import('./platform');
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('IS_MAIN_WINDOW', () => {
  // Catches a flag that is missing or that reads a different label.
  it('is true in the window labelled main', async () => {
    const platform = await load('main');
    expect(platform.IS_MAIN_WINDOW).toBe(true);
    expect(platform.isMainWindow()).toBe(true);
  });

  // Catches a flag that treats every window as main: popouts must never report
  // boot actions.
  it('is false in a popout window', async () => {
    const platform = await load('popout-1');
    expect(platform.IS_MAIN_WINDOW).toBe(false);
    expect(platform.isMainWindow()).toBe(false);
  });

  // Catches a flag that throws or guesses true when Tauri injected nothing.
  it('is false with no Tauri metadata', async () => {
    const platform = await load(undefined);
    expect(platform.IS_MAIN_WINDOW).toBe(false);
  });

  // Catches a module that touches window without checking it exists.
  it('is false with no window at all', async () => {
    vi.resetModules();
    vi.stubGlobal('window', undefined);
    const platform = await import('./platform');
    expect(platform.IS_MAIN_WINDOW).toBe(false);
  });
});
