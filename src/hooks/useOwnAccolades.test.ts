import { describe, expect, it } from 'vitest';
import type { AccoladeWallView } from '../types/profileView';
import { answerOnFailure, isOwnAccoladesChange, mayPaintSaved, ownTzOffsetMin } from './useOwnAccolades';

describe('answerOnFailure', () => {
  const wall: AccoladeWallView = { earned: 1, total: 2, items: [] };

  it('counts a failed read as an answer and keeps this account\'s saved wall', () => {
    expect(answerOnFailure({ userId: 'u1', wall, fresh: false }, 'u1')).toEqual({ userId: 'u1', wall, fresh: true });
  });

  it('answers with nothing when nothing was drawn, or what was drawn was another account\'s', () => {
    expect(answerOnFailure(null, 'u1')).toEqual({ userId: 'u1', wall: null, fresh: true });
    expect(answerOnFailure({ userId: 'u2', wall, fresh: true }, 'u1')).toEqual({ userId: 'u1', wall: null, fresh: true });
  });
});

describe('ownTzOffsetMin', () => {
  it('is east positive: getTimezoneOffset is west positive', () => {
    expect(ownTzOffsetMin({ getTimezoneOffset: () => 480 })).toBe(-480);
    expect(ownTzOffsetMin({ getTimezoneOffset: () => -330 })).toBe(330);
    expect(ownTzOffsetMin({ getTimezoneOffset: () => 0 })).toBe(0);
  });
});

describe('mayPaintSaved', () => {
  const wall: AccoladeWallView = { earned: 0, total: 0, items: [] };

  it('paints a saved wall before the fresh answer', () => {
    expect(mayPaintSaved(false, wall)).toBe(true);
  });

  it('never covers the fresh answer, and never paints a saved nothing', () => {
    expect(mayPaintSaved(true, wall)).toBe(false);
    expect(mayPaintSaved(false, null)).toBe(false);
    expect(mayPaintSaved(false, undefined)).toBe(false);
  });
});

describe('isOwnAccoladesChange', () => {
  it('reads again for this account, and for a payload that names nobody', () => {
    expect(isOwnAccoladesChange({ user_id: '42' }, '42')).toBe(true);
    expect(isOwnAccoladesChange({}, '42')).toBe(true);
    expect(isOwnAccoladesChange(undefined, '42')).toBe(true);
  });

  it('ignores another account', () => {
    expect(isOwnAccoladesChange({ user_id: '7' }, '42')).toBe(false);
  });
});
