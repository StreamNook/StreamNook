// The landscape chat swipe (SN-30): what a release does, where it works, and
// when chat's edge is claimed from Back.
import { describe, expect, it } from 'vitest';
import cases from '../phoneGestureCases.json';
import { backClaimSide, chatSwipeIntent, chatSwipeSide, type ChatSide } from './chatSwipe';

const where = {
  sideBySide: true,
  largeScreen: false,
  largeToggle: false,
  largeChatHidden: false,
  twoColumns: false,
  columnsOnReturn: false,
  overlaySide: 'right' as ChatSide,
};

describe('chat swipe', () => {
  it.each(cases.swipe)('dx $dx dy $dy vx $vx, $side, open $open: $intent', ({ dx, dy, vx, side, open, intent }) => {
    expect(chatSwipeIntent({ dx, dy, vx, side: side as ChatSide, open })).toBe(intent);
  });

  it('works on a phone on its side, on the chat side the overlay setting names', () => {
    expect(chatSwipeSide(where)).toBe('right');
    expect(chatSwipeSide({ ...where, overlaySide: 'left' })).toBe('left');
    expect(chatSwipeSide({ ...where, sideBySide: false })).toBeNull();
  });

  it('works on a large screen only where chat is or returns as columns', () => {
    const large = { ...where, largeScreen: true, largeToggle: true };
    expect(chatSwipeSide({ ...large, twoColumns: true })).toBe('right');
    expect(chatSwipeSide({ ...large, twoColumns: false })).toBeNull();
    expect(chatSwipeSide({ ...large, largeChatHidden: true, columnsOnReturn: true })).toBe('right');
    expect(chatSwipeSide({ ...large, largeChatHidden: true, columnsOnReturn: false })).toBeNull();
    expect(chatSwipeSide({ ...large, largeToggle: false, twoColumns: true })).toBeNull();
  });

  it('claims chat edge from Back only while immersive, chat is away and nothing else takes Back', () => {
    const claim = { immersive: true, side: 'right' as ChatSide | null, chatOpen: false, backTaken: false };
    expect(backClaimSide(claim)).toBe('right');
    expect(backClaimSide({ ...claim, chatOpen: true })).toBeNull();
    expect(backClaimSide({ ...claim, backTaken: true })).toBeNull();
    expect(backClaimSide({ ...claim, side: null })).toBeNull();
    expect(backClaimSide({ ...claim, immersive: false })).toBeNull();
  });
});
