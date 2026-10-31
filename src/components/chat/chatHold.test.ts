import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { armThaw, THAW_BACKSTOP_MS, touchPauseIntent } from './chatHold';

describe('a held row thaws after its click', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('waits for the click, then lets go one tick later', () => {
    const win = new EventTarget();
    const release = vi.fn();
    armThaw(win, release);
    vi.advanceTimersByTime(THAW_BACKSTOP_MS - 1);
    expect(release).not.toHaveBeenCalled();
    win.dispatchEvent(new Event('click'));
    expect(release).not.toHaveBeenCalled();
    vi.advanceTimersByTime(0);
    expect(release).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(THAW_BACKSTOP_MS);
    expect(release).toHaveBeenCalledTimes(1);
  });

  it('lets go at the backstop when no click comes', () => {
    const release = vi.fn();
    armThaw(new EventTarget(), release);
    vi.advanceTimersByTime(THAW_BACKSTOP_MS);
    expect(release).toHaveBeenCalledTimes(1);
  });

  it('a new hold disarms it', () => {
    const win = new EventTarget();
    const release = vi.fn();
    const disarm = armThaw(win, release);
    disarm();
    win.dispatchEvent(new Event('click'));
    vi.advanceTimersByTime(THAW_BACKSTOP_MS * 2);
    expect(release).not.toHaveBeenCalled();
  });

  it('listens on the window in the capture phase, ahead of handlers that stop the click', () => {
    const win = new EventTarget();
    const add = vi.spyOn(win, 'addEventListener');
    armThaw(win, () => {});
    expect(add).toHaveBeenCalledWith('click', expect.any(Function), true);
  });
});

describe('what a touch move means for the pause', () => {
  it('keeps the list rule when the host passes no ratio', () => {
    expect(touchPauseIntent(1, -40, 1)).toBe('pause');
    expect(touchPauseIntent(-1, 0, -5)).toBe('clear');
    expect(touchPauseIntent(0, 0, 0)).toBe('none');
  });

  it('with ratio 1, a sideways drag with downward jitter does not pause, a read-back does', () => {
    expect(touchPauseIntent(1, -12, 2, 1)).toBe('none');
    expect(touchPauseIntent(1, 1, 6, 1)).toBe('pause');
    expect(touchPauseIntent(1, -6, 6, 1)).toBe('none');
  });

  it('with ratio 0.5 (the left grab zone), a thumb arc is not a read-back', () => {
    expect(touchPauseIntent(1, 6, 9, 0.5)).toBe('none');
    expect(touchPauseIntent(1, 4, 9, 0.5)).toBe('pause');
    expect(touchPauseIntent(-1, 4, 9, 0.5)).toBe('clear');
  });
});
