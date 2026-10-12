// A browser drops a whole style rule when any selector in its comma-separated
// list is one it does not know. The desktop app runs on Chromium (WebView2 on
// Windows, CEF on Linux) and on WebKit (WKWebView on macOS). Both know
// `:-webkit-*` and `::-webkit-*` but none of the `-moz-`, `-ms-`, `-o-` or
// `-khtml-` pseudo classes and elements. One `:-moz-full-screen` in a list
// therefore switches off every other selector beside it, silently: the rule is
// valid CSS to the editor, the build and every other check, and simply never
// applies. WebKit before Safari 16.4 (macOS 11 to 13.2) does not know
// unprefixed `:fullscreen` either, so a selector that must always apply never
// shares a list with it.
//
// A rule whose only selector is a non-Chromium pseudo (for example a lone
// `::-moz-range-thumb` rule) loses nothing but itself, so it is allowed here.
//
// The build has the same trap one step earlier. Autoprefixer, set to the same
// Chromium targets, deletes a rule that carries a prefix those targets no
// longer need (`:-webkit-full-screen`) whenever a later rule uses the
// unprefixed form, and every other selector in that rule's list goes with it.
// So each stylesheet is also run through autoprefixer here, and every rule
// with an unprefixed selector must come out the other side.
import { describe, expect, it } from 'vitest';
import autoprefixer from 'autoprefixer';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import postcss, { type AtRule, type Node, type Rule } from 'postcss';

const root = resolve(__dirname, '..', '..');
const src = resolve(root, 'src');

// Pseudo classes and elements behind a vendor prefix Chromium does not parse.
const FOREIGN_PSEUDO = /::?-(?:moz|ms|o|khtml)-[\w-]+/i;

function cssFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...cssFiles(path));
    else if (name.endsWith('.css')) out.push(path);
  }
  return out;
}

// Attribute values and strings can hold any text; only real selector syntax
// should be searched for a pseudo.
function stripQuotedParts(selector: string): string {
  return selector.replace(/"[^"]*"|'[^']*'/g, '""').replace(/\[[^\]]*\]/g, '[]');
}

function insideKeyframes(rule: Rule): boolean {
  for (let node: Node | undefined = rule.parent; node; node = node.parent) {
    if (node.type === 'atrule' && /keyframes$/i.test((node as AtRule).name)) return true;
  }
  return false;
}

function findMixedLists(css: string, from: string): string[] {
  const found: string[] = [];
  postcss.parse(css, { from }).walkRules((rule) => {
    if (insideKeyframes(rule)) return;
    const selectors = rule.selectors;
    if (selectors.length < 2) return;
    const foreign = selectors.filter((s) => FOREIGN_PSEUDO.test(stripQuotedParts(s)));
    if (foreign.length === 0) return;
    const line = rule.source?.start?.line ?? 0;
    found.push(`${from}:${line}  drops [${selectors.join(', ')}] because of [${foreign.join(', ')}]`);
  });
  return found;
}

const VENDOR_PREFIXED = /-(?:webkit|moz|ms|o|khtml)-/i;

function ruleKeys(css: string, from: string): Map<string, number> {
  const keys = new Map<string, number>();
  postcss.parse(css, { from }).walkRules((rule) => {
    if (insideKeyframes(rule)) return;
    keys.set(rule.selectors.map((sel) => sel.replace(/\s+/g, ' ').trim()).join(', '), rule.source?.start?.line ?? 0);
  });
  return keys;
}

// Rules autoprefixer removes although they hold a selector that needs no prefix.
async function findPrefixerDrops(css: string, from: string): Promise<string[]> {
  const out = await postcss([autoprefixer()]).process(css, { from: resolve(root, from) });
  const kept = ruleKeys(out.css, from);
  const found: string[] = [];
  for (const [key, line] of ruleKeys(css, from)) {
    if (kept.has(key)) continue;
    if (key.split(', ').every((sel) => VENDOR_PREFIXED.test(stripQuotedParts(sel)))) continue;
    found.push(`${from}:${line}  autoprefixer deletes [${key}]`);
  }
  return found;
}

describe('CSS selector lists Chromium would drop', () => {
  it('finds the failure mode it guards against', () => {
    const css = '.a:fullscreen .x, .a:-moz-full-screen .x { z-index: 1 }\n'
      + '.b::-moz-range-thumb { color: red }\n'
      + '.c::-webkit-slider-thumb, .c:hover::-webkit-slider-thumb { color: red }\n'
      + '[data-x=":-moz-y"], .d { color: red }\n';
    const hits = findMixedLists(css, 'fixture.css');
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain('fixture.css:1');
  });

  it('no app stylesheet mixes a non-Chromium vendor pseudo into a selector list', () => {
    const files = cssFiles(src);
    expect(files.length).toBeGreaterThan(0);
    const offenders = files.flatMap((file) =>
      findMixedLists(readFileSync(file, 'utf8'), relative(root, file).replace(/\\/g, '/')),
    );
    expect(offenders, `Chromium drops each of these whole rules:\n${offenders.join('\n')}`).toEqual([]);
  });

  it('no rule mixes unprefixed :fullscreen with selectors old WebKit keeps', () => {
    const FULLSCREEN = /:fullscreen\b/;
    const mixed = (css: string, from: string): string[] => {
      const found: string[] = [];
      postcss.parse(css, { from }).walkRules((rule) => {
        if (insideKeyframes(rule) || rule.selectors.length < 2) return;
        const uses = rule.selectors.map((s) => FULLSCREEN.test(stripQuotedParts(s)));
        if (uses.some(Boolean) && !uses.every(Boolean)) {
          found.push(`${from}:${rule.source?.start?.line ?? 0}  [${rule.selectors.join(', ')}]`);
        }
      });
      return found;
    };
    expect(mixed('.a:fullscreen .x, .b ~ .x { top: 0 }\n.c:fullscreen .y { top: 0 }\n', 'fixture.css')).toHaveLength(1);
    const offenders = cssFiles(src).flatMap((file) =>
      mixed(readFileSync(file, 'utf8'), relative(root, file).replace(/\\/g, '/')),
    );
    expect(offenders, `macOS 11 to 13.2 drops each of these whole rules:\n${offenders.join('\n')}`).toEqual([]);
  });

  it('catches a rule autoprefixer would delete with its whole list', async () => {
    const css = '.a:fullscreen .s, .a:-webkit-full-screen .s, .b ~ .s { z-index: 1 }\n'
      + '.a:fullscreen .t { z-index: 2 }\n';
    const hits = await findPrefixerDrops(css, 'src/styles/fixture.css');
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain('fixture.css:1');
  });

  it('autoprefixer keeps every app rule that holds an unprefixed selector', async () => {
    const offenders: string[] = [];
    for (const file of cssFiles(src)) {
      const name = relative(root, file).replace(/\\/g, '/');
      offenders.push(...(await findPrefixerDrops(readFileSync(file, 'utf8'), name)));
    }
    expect(offenders, `The build deletes each of these whole rules:\n${offenders.join('\n')}`).toEqual([]);
  });
});
