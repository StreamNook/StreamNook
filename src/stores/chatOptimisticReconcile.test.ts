import { describe, expect, it } from 'vitest';
import { DUPLICATE_BYPASS_SUFFIX, sameSentContent, varyRepeat } from '../utils/sentContent';

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

// Twitch refuses a line identical to the one right before it. Repeats used to
// carry the suffix every time, so the third copy matched the second and was
// dropped ("it only works for 2 messages").
describe('varyRepeat', () => {
  const varied = `hi${DUPLICATE_BYPASS_SUFFIX}`;

  it('alternates plain and varied so no two sends in a row match', () => {
    let last: string | undefined;
    const sent: string[] = [];
    for (let i = 0; i < 5; i++) {
      const out = varyRepeat('hi', last);
      sent.push(out);
      last = out;
    }
    expect(sent).toEqual(['hi', varied, 'hi', varied, 'hi']);
    for (let i = 1; i < sent.length; i++) expect(sent[i]).not.toBe(sent[i - 1]);
  });

  it('leaves a different line alone', () => {
    expect(varyRepeat('hello', 'hi')).toBe('hello');
    expect(varyRepeat('hello', varied)).toBe('hello');
  });

  it('treats the trailing space the emote picker leaves as the same line', () => {
    expect(varyRepeat('catJAM ', 'catJAM')).toBe(`catJAM${DUPLICATE_BYPASS_SUFFIX}`);
    expect(varyRepeat('catJAM ', `catJAM${DUPLICATE_BYPASS_SUFFIX}`)).toBe('catJAM');
  });
});
