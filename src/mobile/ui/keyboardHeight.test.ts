import { describe, expect, it } from 'vitest';
import { DEFAULT_KEYBOARD_PX, rememberedHeight } from './keyboardHeight';

describe('rememberedHeight', () => {
  it('keeps the last open keyboard height', () => {
    expect(rememberedHeight(DEFAULT_KEYBOARD_PX, 312.4)).toBe(312);
    expect(rememberedHeight(312, 0)).toBe(312);
  });
  it('ignores a toolbar-sized reading', () => {
    expect(rememberedHeight(312, 48)).toBe(312);
  });
});
