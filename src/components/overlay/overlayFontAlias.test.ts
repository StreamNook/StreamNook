// Run with: npm test
//
// The installed-font alias puts a streamer's typed font name into CSS text (an
// @font-face rule set as a <style> element's text). These tests tokenize that
// CSS the way a browser does and prove no name, however crafted, can end a
// string early, open a second rule, or carry markup out of the rule.

import { describe, expect, it } from 'vitest';

import {
  FONT_OPTIONS,
  localFontAlias,
  localFontFaceCss,
  overlayFontStack,
  primaryFontFamily,
  splitFontFamilies,
  touchLocalFontAliases,
} from './overlayConfig';

interface Scan {
  /** Text outside strings, with every string replaced by a "S" placeholder. */
  outside: string;
  /** Every string token's value, escapes resolved. */
  strings: string[];
}

/** A minimal CSS tokenizer: string tokens with backslash escapes, everything
 *  else kept verbatim. Throws on a string that a newline or the end cuts off,
 *  the cases where CSS makes a bad-string and the rest re-parses as code. */
function scanCss(css: string): Scan {
  let outside = '';
  const strings: string[] = [];
  let i = 0;
  while (i < css.length) {
    const ch = css[i];
    if (ch !== "'" && ch !== '"') {
      outside += ch;
      i++;
      continue;
    }
    const quote = ch;
    let value = '';
    i++;
    for (;;) {
      if (i >= css.length) throw new Error(`unterminated string in ${css}`);
      const c = css[i];
      if (c === quote) { i++; break; }
      if (c === '\n' || c === '\r' || c === '\f') throw new Error(`newline inside a string in ${JSON.stringify(css)}`);
      if (c === '\\') {
        const next = css[i + 1];
        if (next === undefined) throw new Error('escape at the end of the input');
        const hex = /^[0-9a-fA-F]{1,6}/.exec(css.slice(i + 1));
        if (hex) {
          value += String.fromCodePoint(parseInt(hex[0], 16));
          i += 1 + hex[0].length;
          if (css[i] === ' ') i++;
        } else {
          value += next;
          i += 2;
        }
        continue;
      }
      value += c;
      i++;
    }
    strings.push(value);
    outside += 'S';
  }
  return { outside, strings };
}

const NASTY_NAMES = [
  "Bob's Font",
  "x'); } body { background: red } @font-face { font-family: 'y",
  '</style><script>alert(1)</script>',
  'a\\\'b',
  'trailing backslash \\',
  'line\nbreak',
  'carriage\rreturn',
  'form\ffeed',
  'double "quoted" name',
  'a\\41 hex-looking escape',
  '游ゴシック Medium',
  '  padded  ',
];

/** The names localFontFaceCss looks up, the way it builds them. */
const expectedLocalNames = (family: string): string[] => {
  const compact = family.replace(/\s+/g, '');
  const names = Array.from(new Set([family, `${family} Regular`, compact, `${compact}-Regular`]));
  // A newline cannot sit in a CSS string; the rule carries a space instead.
  return names.map((n) => n.replace(/[\n\r\f]/g, ' '));
};

describe('installed-font alias CSS', () => {
  for (const name of NASTY_NAMES) {
    it(`stays one @font-face rule for ${JSON.stringify(name)}`, () => {
      const css = localFontFaceCss(name);
      const { outside, strings } = scanCss(css);
      // Exactly the rule's own shape outside the strings: nothing the name
      // typed can show up as code.
      const locals = expectedLocalNames(name);
      expect(outside).toBe(`@font-face{font-family:S;src:${locals.map(() => 'local(S)').join(',')};font-weight:100 900;}`);
      expect(strings[0]).toBe(localFontAlias(name));
      expect(strings.slice(1)).toEqual(locals);
      // Set as text, never parsed as HTML, but keep markup out of code anyway.
      expect(outside).not.toMatch(/[<>]/);
    });
  }

  it('names the alias with a CSS-safe identifier', () => {
    for (const name of NASTY_NAMES) {
      expect(localFontAlias(name)).toMatch(/^sn-local-[a-z0-9-]*$/);
    }
  });

  it('only ever inserts the quoted alias into a font stack', () => {
    for (const name of NASTY_NAMES.filter((n) => !/[,'"]/.test(n))) {
      const stack = `'${name}', sans-serif`;
      const out = overlayFontStack(stack);
      if (splitFontFamilies(stack) === null) {
        // A trailing backslash escapes the closing quote, so CSS reads the
        // string as unclosed; the stack is left exactly as it was.
        expect(out).toBe(stack);
        continue;
      }
      expect(out).toBe(`'${name}', '${localFontAlias(primaryFontFamily(stack))}', sans-serif`);
    }
  });

  it('gives every distinct name its own alias, non-Latin names included', () => {
    const names = ['メイリオ', '游ゴシック', '游ゴシック Medium', '游明朝 Medium', 'Montserrat Bold', 'montserrat bold', 'Montserrat-Bold'];
    const aliases = names.map(localFontAlias);
    expect(new Set(aliases).size).toBe(names.length);
    expect(localFontAlias('Montserrat Bold')).toBe(localFontAlias('Montserrat Bold'));
  });

  it('claims every weight, so a Bold face is never thickened again', () => {
    expect(localFontFaceCss('Montserrat Bold')).toContain('font-weight:100 900;');
  });

  it('splits a stack at the commas outside quotes', () => {
    expect(splitFontFamilies("'A, B', \"C\", sans-serif")).toEqual(["'A, B'", '"C"', 'sans-serif']);
    expect(primaryFontFamily("'A, B', sans-serif")).toBe('A, B');
    expect(overlayFontStack("'A, B', sans-serif")).toBe(`'A, B', '${localFontAlias('A, B')}', sans-serif`);
    // What the custom-font box stores for a moment when a comma is typed.
    expect(overlayFontStack("'Poppins,', sans-serif")).toBe(`'Poppins,', '${localFontAlias('Poppins,')}', sans-serif`);
    // An unclosed quote is left exactly as it was.
    expect(splitFontFamilies("'Poppins, sans-serif")).toBeNull();
    expect(overlayFontStack("'Poppins, sans-serif")).toBe("'Poppins, sans-serif");
    // Unquoted multi-word names keep working.
    expect(overlayFontStack('Uni Sans, serif')).toBe(`Uni Sans, '${localFontAlias('Uni Sans')}', serif`);
  });

  it('keeps the most recently used aliases, oldest out first', () => {
    let cache: string[] = [];
    for (const f of ['a', 'b', 'c']) cache = touchLocalFontAliases(cache, f, 3);
    expect(cache).toEqual(['a', 'b', 'c']);
    // Using 'a' again makes it the newest, so 'b' is the one that gives way.
    cache = touchLocalFontAliases(cache, 'a', 3);
    cache = touchLocalFontAliases(cache, 'd', 3);
    expect(cache).toEqual(['c', 'a', 'd']);
    for (let i = 0; i < 40; i++) cache = touchLocalFontAliases(cache, `prefix ${i}`, 16);
    expect(cache.length).toBe(16);
    expect(cache[cache.length - 1]).toBe('prefix 39');
  });

  it('keeps every preset stack in order with only the alias added', () => {
    for (const f of FONT_OPTIONS) {
      const out = overlayFontStack(f.value);
      const before = f.value.split(',').map((p) => p.trim());
      const after = out.split(',').map((p) => p.trim());
      const withoutAlias = after.filter((p) => !p.startsWith("'sn-local-"));
      expect(withoutAlias).toEqual(before);
      expect(after.length - before.length).toBeLessThanOrEqual(1);
    }
  });
});
