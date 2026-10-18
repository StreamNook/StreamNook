import { describe, expect, it } from 'vitest';
import { isPalindrome, outOfReachAccolades } from './accoladeReach';

const EVENTS = ['cs2_major_cologne_2026', 'semiquincentennial_2026', 'grand_finale_2026'];

describe('outOfReachAccolades', () => {
  it('closes the finished events for everyone', () => {
    for (const n of [1, 67, 167, 5000, null]) {
      for (const id of EVENTS) expect(outOfReachAccolades(n).has(id)).toBe(true);
    }
  });

  it('closes OG past the first 100 members', () => {
    expect(outOfReachAccolades(100).has('og')).toBe(false);
    expect(outOfReachAccolades(101).has('og')).toBe(true);
  });

  it('closes Palindrome and Nice for a number that does not qualify', () => {
    const closed = outOfReachAccolades(168);
    expect(closed.has('palindrome')).toBe(true);
    expect(closed.has('nice')).toBe(true);
  });

  it('keeps open the number accolades a member already qualifies for', () => {
    expect(outOfReachAccolades(121).has('palindrome')).toBe(false);
    expect(outOfReachAccolades(167).has('nice')).toBe(false);
    expect(outOfReachAccolades(69).has('nice')).toBe(false);
  });

  it('closes every number accolade while the number is unknown', () => {
    const closed = outOfReachAccolades(null);
    for (const id of ['og', 'palindrome', 'nice']) expect(closed.has(id)).toBe(true);
  });

  it('never closes an accolade anyone can still work toward', () => {
    const closed = outOfReachAccolades(5000);
    for (const id of ['chatterbox', 'veteran', 'ancient', 'completionist', 'triple', 'insomniac', 'spring', 'cake_day']) {
      expect(closed.has(id)).toBe(false);
    }
  });
});

describe('isPalindrome', () => {
  it('needs two digits that read the same both ways', () => {
    expect(isPalindrome(7)).toBe(false);
    expect(isPalindrome(11)).toBe(true);
    expect(isPalindrome(121)).toBe(true);
    expect(isPalindrome(123)).toBe(false);
    expect(isPalindrome(null)).toBe(false);
  });
});
