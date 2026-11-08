import { describe, expect, it } from 'vitest';
import { CHAT_SEPARATOR, compactChatPlacement, compactChatSlot } from './compactView';

describe('Compact View chat placement', () => {
  it('hides chat unless the viewer keeps it', () => {
    expect(compactChatPlacement('right', false)).toBe('hidden');
    expect(compactChatPlacement('bottom', false)).toBe('hidden');
  });

  it('keeps chat where it was docked when kept', () => {
    expect(compactChatPlacement('right', true)).toBe('right');
    expect(compactChatPlacement('left', true)).toBe('left');
    expect(compactChatPlacement('bottom', true)).toBe('bottom');
    expect(compactChatPlacement('hidden', true)).toBe('hidden');
  });
});

describe('Compact View chat slot', () => {
  const base = { chatSize: 402, revealOnHover: false, chatInPopout: false };

  it('takes side chat and its divider out of the width', () => {
    expect(compactChatSlot({ ...base, placement: 'right' })).toEqual({
      placement: 'right',
      chatSize: 402,
      uiWidthOffset: CHAT_SEPARATOR,
      uiHeightOffset: 0,
    });
    expect(compactChatSlot({ ...base, placement: 'left' })?.uiWidthOffset).toBe(CHAT_SEPARATOR);
  });

  it('takes bottom chat and its divider out of the height', () => {
    expect(compactChatSlot({ ...base, chatSize: 200, placement: 'bottom' })).toEqual({
      placement: 'bottom',
      chatSize: 200,
      uiWidthOffset: 0,
      uiHeightOffset: CHAT_SEPARATOR,
    });
  });

  it('takes no room when chat is hidden, reveals on hover, or lives in a popout', () => {
    expect(compactChatSlot({ ...base, placement: 'hidden' })).toBeNull();
    expect(compactChatSlot({ ...base, placement: 'right', revealOnHover: true })).toBeNull();
    expect(compactChatSlot({ ...base, placement: 'right', chatInPopout: true })).toBeNull();
    expect(compactChatSlot({ ...base, placement: 'right', chatSize: 0 })).toBeNull();
  });

  it('keeps bottom chat even with reveal on hover, which only the side docks have', () => {
    expect(compactChatSlot({ ...base, chatSize: 200, placement: 'bottom', revealOnHover: true })).not.toBeNull();
  });
});
