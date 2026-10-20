// Hiding a pinned message on the phone: each room starts as Chat > Pinned
// Messages > Collapsed says, a tap holds until the chat switches, and Style:
// Hidden draws nothing over chat while the pin is folded (the desktop's rule;
// the phone once left a pin button floating there).
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../../stores/AppStore', () => ({ useAppStore: vi.fn() }));
vi.mock('../../stores/pinStore', () => ({ usePinStore: vi.fn() }));
vi.mock('../chat/ChatTabStrip', () => ({ useActiveChatChannelId: vi.fn() }));
vi.mock('../../components/chat/PinnedChatter', () => ({ PinnedChatter: () => null }));

import { pinsOpenFor } from './pinsOpen';
import { PinnedBanner, type PinnedItem } from './PinnedBanner';

const pin: PinnedItem = {
  id: 'p1',
  message_id: 'm1',
  message_text: 'Giveaway at 9',
  sender_name: 'Streamer',
  sender_color: '#9146ff',
};

describe('pinned banner open state', () => {
  it('starts each room as Collapsed says', () => {
    expect(pinsOpenFor(null, '123', true)).toBe(false);
    expect(pinsOpenFor(null, '123', false)).toBe(true);
  });

  it("holds a room's own tap until the chat switches", () => {
    const folded = { channel: '123', open: false };
    expect(pinsOpenFor(folded, '123', false)).toBe(false);
    expect(pinsOpenFor(folded, '456', false)).toBe(true);
    expect(pinsOpenFor({ channel: '123', open: true }, '123', true)).toBe(true);
  });
});

describe('Pinned Messages > Style', () => {
  const draw = (expanded: boolean, collapsedStyle: 'bar' | 'hidden') =>
    renderToStaticMarkup(
      createElement(PinnedBanner, { pins: [pin], expanded, onToggle: () => {}, collapsedStyle }),
    );

  it('draws nothing for a folded pin when Hidden', () => {
    expect(draw(false, 'hidden')).toBe('');
  });

  it('keeps the bar and the open pin drawn', () => {
    expect(draw(false, 'bar')).toContain('Giveaway at 9');
    expect(draw(true, 'hidden')).toContain('Giveaway at 9');
  });
});
