import { describe, expect, it } from 'vitest';
import { DEFAULT_KEYBOARD_PX, EMOTE_PANEL_MIN_PX, emotePanelHeight, rememberedHeight } from './keyboardHeight';

describe('emotePanelHeight', () => {
  it('takes a tall keyboard height as it is', () => {
    expect(emotePanelHeight(342.6)).toBe(343);
  });
  it('never opens shorter than a panel you can browse', () => {
    // A 250px keyboard (gesture bar already off) left the grid one cut-off row.
    expect(emotePanelHeight(250)).toBe(EMOTE_PANEL_MIN_PX);
    expect(emotePanelHeight(DEFAULT_KEYBOARD_PX)).toBe(EMOTE_PANEL_MIN_PX);
  });
});

describe('rememberedHeight', () => {
  it('keeps the last open keyboard height', () => {
    expect(rememberedHeight(DEFAULT_KEYBOARD_PX, 312.4)).toBe(312);
    expect(rememberedHeight(312, 0)).toBe(312);
  });
  it('ignores a toolbar-sized reading', () => {
    expect(rememberedHeight(312, 48)).toBe(312);
  });
});
