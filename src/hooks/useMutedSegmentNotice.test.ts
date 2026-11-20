import { describe, expect, it } from 'vitest';
import { mutedRangeAt, skipMutedTarget } from './useMutedSegmentNotice';

const ranges = [
  { start_secs: 60, end_secs: 240 },
  { start_secs: 600, end_secs: 780 },
];

describe('mutedRangeAt', () => {
  it('finds the range the time is inside, start inclusive and end exclusive', () => {
    expect(mutedRangeAt(ranges, 60)).toEqual(ranges[0]);
    expect(mutedRangeAt(ranges, 239.9)).toEqual(ranges[0]);
    expect(mutedRangeAt(ranges, 240)).toBeNull();
    expect(mutedRangeAt(ranges, 700)).toEqual(ranges[1]);
  });

  it('is null with no ranges or no time', () => {
    expect(mutedRangeAt(undefined, 100)).toBeNull();
    expect(mutedRangeAt([], 100)).toBeNull();
    expect(mutedRangeAt(ranges, Number.NaN)).toBeNull();
  });
});

describe('skipMutedTarget (SN-282)', () => {
  it('lands where the sound comes back', () => {
    expect(skipMutedTarget(ranges[0], 3600)).toBe(240);
  });

  it('stops just short of the end when the mute runs to the end', () => {
    expect(skipMutedTarget({ start_secs: 3000, end_secs: 3600 }, 3600)).toBe(3599.5);
  });

  it('keeps the range end while the length is unknown', () => {
    expect(skipMutedTarget(ranges[1], Number.NaN)).toBe(780);
    expect(skipMutedTarget(ranges[1], 0)).toBe(780);
  });
});
