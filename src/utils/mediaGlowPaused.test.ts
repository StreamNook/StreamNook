import { describe, expect, it } from 'vitest';
import { pausedFrameAlreadyRead } from './mediaGlowPaused';

describe('pausedFrameAlreadyRead', () => {
  it('skips only a paused frame already read', () => {
    expect(pausedFrameAlreadyRead({ paused: true, seeking: false, currentTime: 5 }, 5)).toBe(true);
    expect(pausedFrameAlreadyRead({ paused: true, seeking: false, currentTime: 6 }, 5)).toBe(false);
    expect(pausedFrameAlreadyRead({ paused: true, seeking: true, currentTime: 5 }, 5)).toBe(false);
    expect(pausedFrameAlreadyRead({ paused: false, seeking: false, currentTime: 5 }, 5)).toBe(false);
    expect(pausedFrameAlreadyRead({ paused: true, seeking: false, currentTime: 5 }, Number.NaN)).toBe(false);
  });
});
