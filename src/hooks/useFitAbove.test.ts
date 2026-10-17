import { describe, expect, it } from 'vitest';
import { roomAbove } from './useFitAbove';

describe('roomAbove', () => {
  it('keeps the full height when there is room', () => {
    expect(roomAbove(1278, 40, 520)).toBe(520);
  });

  it('shrinks to the space between the menu and the column top', () => {
    // A short window: the menu's bottom sits 400 px below the column top.
    expect(roomAbove(440, 40, 520)).toBe(392);
  });

  it('never goes below a usable minimum', () => {
    expect(roomAbove(120, 40, 520)).toBe(160);
  });
});
