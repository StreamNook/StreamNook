import { describe, expect, it } from 'vitest';

import { pickerReasons, REASON_MAX, reasonKey } from './reasonPick';

const reasons = ['Spam', 'Hate speech', 'Self-promotion'];
const outside = { inField: false, text: '', reasons, modified: false };
const inField = (text: string) => ({ inField: true, text, reasons, modified: false });

describe('reasonKey', () => {
  it('a digit picks the reason numbered with it', () => {
    expect(reasonKey('1', outside)).toEqual({ kind: 'pick', reason: 'Spam' });
    expect(reasonKey('3', outside)).toEqual({ kind: 'pick', reason: 'Self-promotion' });
  });

  it('a digit past the saved reasons does nothing', () => {
    expect(reasonKey('4', outside)).toBeNull();
    expect(reasonKey('9', { ...outside, reasons: [] })).toBeNull();
  });

  it('0 is No reason', () => {
    expect(reasonKey('0', outside)).toEqual({ kind: 'pick', reason: null });
  });

  it('digits typed in the field are text, not picks', () => {
    expect(reasonKey('1', inField(''))).toBeNull();
    expect(reasonKey('0', inField('rule '))).toBeNull();
  });

  it('a digit with a modifier held is left alone', () => {
    expect(reasonKey('1', { ...outside, modified: true })).toBeNull();
  });

  it('Enter sends the written reason, trimmed and capped', () => {
    expect(reasonKey('Enter', inField('  being rude  '))).toEqual({ kind: 'pick', reason: 'being rude' });
    const long = 'x'.repeat(REASON_MAX + 40);
    expect(reasonKey('Enter', inField(long))).toEqual({ kind: 'pick', reason: 'x'.repeat(REASON_MAX) });
  });

  it('Enter with nothing written does nothing', () => {
    expect(reasonKey('Enter', inField('   '))).toBeNull();
    expect(reasonKey('Enter', outside)).toBeNull();
  });

  it('Esc closes from anywhere', () => {
    expect(reasonKey('Escape', outside)).toEqual({ kind: 'close' });
    expect(reasonKey('Escape', inField('half a reas'))).toEqual({ kind: 'close' });
  });

  it('a letter outside the field starts a written reason', () => {
    expect(reasonKey('b', outside)).toEqual({ kind: 'type' });
    expect(reasonKey(' ', outside)).toBeNull();
    expect(reasonKey('Tab', outside)).toBeNull();
  });
});

describe('pickerReasons', () => {
  it('drops blanks and repeats and keeps nine in order', () => {
    const saved = [' Spam ', '', 'Spam', ...Array.from({ length: 12 }, (_, i) => `r${i}`)];
    const out = pickerReasons(saved);
    expect(out).toHaveLength(9);
    expect(out[0]).toBe('Spam');
    expect(out[1]).toBe('r0');
  });

  it('is empty with nothing saved', () => {
    expect(pickerReasons(undefined)).toEqual([]);
  });
});
