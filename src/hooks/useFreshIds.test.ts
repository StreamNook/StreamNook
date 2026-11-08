// @vitest-environment node
// Run with: npm test
import { describe, expect, it } from 'vitest';
import { freshAgainst } from './useFreshIds';

const sig = (ids: string[]) => ids.join('\n');

describe('freshAgainst', () => {
  it('marks only the id that replaced another', () => {
    expect([...freshAgainst(sig(['tw-1', 'sn-x']), ['tw-2', 'sn-x'])]).toEqual(['tw-2']);
  });

  it('marks nothing when a badge is only taken off', () => {
    expect([...freshAgainst(sig(['a', 'b']), ['a'])]).toEqual([]);
  });

  it('treats an id put back after removal as fresh', () => {
    expect([...freshAgainst(sig(['a']), ['a', 'b'])]).toEqual(['b']);
  });

  it('marks everything against an empty list', () => {
    expect([...freshAgainst('', ['paint-1'])]).toEqual(['paint-1']);
  });
});
