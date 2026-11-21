// SN-284 D16: what a held pins block draws.
import { describe, expect, it } from 'vitest';
import { dayLabelFor, mergeHeld } from './pinsHold';
import { samplePin } from '../settings/pinSamples';

const pin = (n: number, rev = n) => ({ ...samplePin(n, 'mention', `u${n}`, '#fff', `m${n}`, '9:00 PM'), rev });
const ids = (list: Array<{ entry: { id: string } }>) => list.map((p) => p.entry.id);

describe('mergeHeld', () => {
  it('keeps a pin Rust ended while held', () => {
    const frozen = [pin(1), pin(2), pin(3)];
    const live = [pin(2), pin(3)];
    expect(ids(mergeHeld(frozen, live))).toEqual(ids(frozen));
  });

  it('shows a pin that arrives at the newest end', () => {
    const frozen = [pin(1), pin(2)];
    const live = [pin(1), pin(2), pin(3)];
    expect(ids(mergeHeld(frozen, live))).toEqual(ids([pin(1), pin(2), pin(3)]));
  });

  it('queues an insert above what is held until release', () => {
    const frozen = [pin(2), pin(3)];
    const live = [pin(1), pin(2), pin(3)];
    expect(ids(mergeHeld(frozen, live))).toEqual(ids([pin(2), pin(3)]));
  });

  it('takes the newest data of a held pin, in its place', () => {
    const frozen = [pin(1, 1), pin(2, 2)];
    const live = [pin(1, 9), pin(2, 2)];
    const merged = mergeHeld(frozen, live);
    expect(merged[0].rev).toBe(9);
    expect(ids(merged)).toEqual(ids(frozen));
  });
});

describe('dayLabelFor', () => {
  it('is null for today and a day name or date otherwise', () => {
    const now = new Date(2026, 10, 21, 15, 0, 0);
    expect(dayLabelFor(new Date(2026, 10, 21, 9, 0, 0).getTime(), now, 'en-US')).toBeNull();
    expect(dayLabelFor(new Date(2026, 10, 19, 9, 0, 0).getTime(), now, 'en-US')).toBe('Thu');
    expect(dayLabelFor(new Date(2026, 9, 2, 9, 0, 0).getTime(), now, 'en-US')).toBe('Oct 2');
  });
});
