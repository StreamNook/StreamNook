// Run with: npm test
//
// A closed approval sheet stays quiet; every other follow failure still shows.

import { describe, expect, it } from 'vitest';
import { isApprovalClosed } from './twitchApproval';

describe('isApprovalClosed', () => {
  it('recognizes the closed sheet however the rejection arrives', () => {
    expect(isApprovalClosed('APPROVAL_CLOSED')).toBe(true);
    expect(isApprovalClosed(new Error('APPROVAL_CLOSED'))).toBe(true);
  });

  it('leaves real failures alone', () => {
    expect(isApprovalClosed('GQL follow failed (HTTP 500): oops')).toBe(false);
    expect(isApprovalClosed(new Error('NEEDS_DROPS_AUTH'))).toBe(false);
    expect(isApprovalClosed(undefined)).toBe(false);
  });
});
