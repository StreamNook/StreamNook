import { describe, expect, it } from 'vitest';
import { sameSentContent } from '../utils/sentContent';

// The emote picker leaves "name " in the compose box; Twitch echoes "name".
// Before this helper the optimistic row and the echo compared unequal whenever
// the IRC echo beat the Helix id stamp, and your own message showed twice.
describe('sameSentContent', () => {
  it('ignores the trailing space the emote picker leaves behind', () => {
    expect(sameSentContent('catJAM ', 'catJAM')).toBe(true);
    expect(sameSentContent('hello catJAM  ', 'hello catJAM')).toBe(true);
  });

  it('still matches exact content', () => {
    expect(sameSentContent('hello', 'hello')).toBe(true);
  });

  it('keeps the duplicate-bypass suffix significant', () => {
    const bypassed = 'hello \u{E0000}';
    expect(sameSentContent(bypassed, bypassed)).toBe(true);
    expect(sameSentContent(bypassed, 'hello')).toBe(false);
  });

  it('does not treat different text or non-string echoes as a match', () => {
    expect(sameSentContent('hello there', 'hello')).toBe(false);
    expect(sameSentContent('a  b', 'a b')).toBe(false);
    expect(sameSentContent('hello', undefined)).toBe(false);
  });
});
