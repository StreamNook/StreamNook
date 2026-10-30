// @vitest-environment node
// Haptics go to the Android shell's own touch feedback when it is there (the
// WebView's navigator.vibrate is silent without a permission the app lacks),
// and to navigator.vibrate everywhere else.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { hapticArm, hapticCommit, hapticDestructive, hapticStep, hapticTick } from './haptics';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('haptics', () => {
  it('asks the Android shell for touch feedback when it is there', () => {
    const haptic = vi.fn();
    const vibrate = vi.fn();
    vi.stubGlobal('window', { SNInsets: { haptic } });
    vi.stubGlobal('navigator', { vibrate });
    hapticArm();
    hapticTick();
    hapticStep();
    hapticCommit();
    hapticDestructive();
    expect(haptic.mock.calls.map((call) => call[0])).toEqual(['arm', 'tick', 'step', 'commit', 'destructive']);
    expect(vibrate).not.toHaveBeenCalled();
  });

  it('falls back to the web call without the shell', () => {
    const vibrate = vi.fn();
    vi.stubGlobal('window', {});
    vi.stubGlobal('navigator', { vibrate });
    hapticTick();
    hapticDestructive();
    expect(vibrate.mock.calls).toEqual([[8], [[15, 40, 15]]]);
  });

  it('never throws when neither is there', () => {
    vi.stubGlobal('window', undefined);
    vi.stubGlobal('navigator', undefined);
    expect(() => hapticCommit()).not.toThrow();
  });
});
