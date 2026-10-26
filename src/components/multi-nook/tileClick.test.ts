import { describe, expect, it } from 'vitest';
import { tileClickAction } from './tileClick';

const base = { onControl: false, isMaximized: false, isFocused: false, detail: 1 };

describe('tileClickAction', () => {
  it('ignores a click on a control, in every tile state', () => {
    for (const isMaximized of [false, true]) {
      for (const isFocused of [false, true]) {
        expect(tileClickAction({ ...base, onControl: true, isMaximized, isFocused })).toBe('ignore');
      }
    }
  });

  it('ignores the second click of a double-click, in every tile state', () => {
    for (const detail of [2, 3]) {
      for (const isMaximized of [false, true]) {
        for (const isFocused of [false, true]) {
          expect(tileClickAction({ ...base, detail, isMaximized, isFocused })).toBe('ignore');
        }
      }
    }
  });

  it('toggles play later on a maximized tile, focused or not', () => {
    expect(tileClickAction({ ...base, isMaximized: true, isFocused: true })).toBe('toggle-play-later');
    expect(tileClickAction({ ...base, isMaximized: true, isFocused: false })).toBe('toggle-play-later');
  });

  it('focuses an unfocused tile at once', () => {
    expect(tileClickAction({ ...base, isFocused: false })).toBe('focus');
  });

  it('unfocuses a focused tile later', () => {
    expect(tileClickAction({ ...base, isFocused: true })).toBe('unfocus-later');
  });

  it('leaves a maximized tile playing when Click to pause is off', () => {
    expect(tileClickAction({ ...base, isMaximized: true, clickToPause: false })).toBe('ignore');
    expect(tileClickAction({ ...base, isMaximized: true, clickToPause: true })).toBe('toggle-play-later');
  });

  it('still switches the audio tile with Click to pause off', () => {
    expect(tileClickAction({ ...base, isFocused: false, clickToPause: false })).toBe('focus');
    expect(tileClickAction({ ...base, isFocused: true, clickToPause: false })).toBe('unfocus-later');
  });
});
