import { describe, expect, it } from 'vitest';
import { blockedTermProblem, filterBlockedTerms, type BlockedTermRow } from './blockedTerms';

const row = (id: string, text: string): BlockedTermRow => ({
  id,
  text,
  added: 'Added 1 day ago',
  expires: null,
  created_at_ms: 0,
  expires_at_ms: null,
});

describe('blockedTermProblem', () => {
  it('takes 2 to 500 characters, counted after trimming', () => {
    expect(blockedTermProblem('a')).toBe('Use at least 2 characters.');
    expect(blockedTermProblem('  a  ')).toBe('Use at least 2 characters.');
    expect(blockedTermProblem('ab')).toBeNull();
    expect(blockedTermProblem('spam*')).toBeNull();
    expect(blockedTermProblem('x'.repeat(500))).toBeNull();
    expect(blockedTermProblem('x'.repeat(501))).toBe('Keep it to 500 characters or fewer.');
  });

  it('counts an emoji as one character', () => {
    expect(blockedTermProblem('😀😀')).toBeNull();
    expect(blockedTermProblem('😀')).toBe('Use at least 2 characters.');
  });
});

describe('filterBlockedTerms', () => {
  const rows = [row('1', 'Spam*'), row('2', 'free followers'), row('3', 'scam link')];

  it('keeps every row for an empty search', () => {
    expect(filterBlockedTerms(rows, '')).toBe(rows);
    expect(filterBlockedTerms(rows, '   ')).toBe(rows);
  });

  it('matches part of a phrase, ignoring case', () => {
    expect(filterBlockedTerms(rows, 'SPAM').map((r) => r.id)).toEqual(['1']);
    expect(filterBlockedTerms(rows, 'am').map((r) => r.id)).toEqual(['1', '3']);
    expect(filterBlockedTerms(rows, 'nothing')).toEqual([]);
  });
});
