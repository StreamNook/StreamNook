import { describe, expect, it } from 'vitest';
import { opaqueFloors, PHONE_FLOOR, WINDOW_FLOOR } from './index';

describe('opaque floors', () => {
  it('flattens a see-through background over the window and tertiary over the page', () => {
    expect(opaqueFloors('rgba(40, 60, 80, 0.5)', 'rgba(200, 200, 200, 0.5)', WINDOW_FLOOR))
      .toEqual({ background: '#1a242f', tertiary: '#71767c' });
  });
  it('flattens over black on a phone', () => {
    expect(opaqueFloors('rgba(40, 60, 80, 0.5)', 'rgba(200, 200, 200, 0.5)', PHONE_FLOOR))
      .toEqual({ background: '#141e28', tertiary: '#6e7378' });
  });
  it('leaves opaque floors as they are', () => {
    expect(opaqueFloors('#0c0c0d', '#1a1a1b', WINDOW_FLOOR)).toEqual({ background: '#0c0c0d', tertiary: '#1a1a1b' });
  });
});
