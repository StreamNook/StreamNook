// Glassiness 0% means no see-through surfaces. The slider only reaches a
// surface through three doors: the --color-surface* tokens it rewrites, the
// .glass-* classes that fade an opaque base in with --glass-strength, and the
// `glass` Tailwind colours (bg-glass-ink/90 is 90% at full glass and opaque at
// 0%). Any other translucent fill keeps its alpha at 0% and loses its frost
// too, so it goes see-through AND sharp. Users report that daily, so this
// test reads the source and fails on the patterns that cause it.
//
// What counts as a surface: something positioned over other content (fixed,
// absolute or sticky) or frosted (backdrop blur). Something in flow sits on
// its parent, which is the parent's job. Scrims (inset-0 dimmers behind a
// dialog or over a video tile) and thin decorations (1-4px lines,
// pointer-events-none marks) are not surfaces.
//
// A deliberate exception carries `glass-exempt: <reason>` in a comment within
// the four lines above its className, or is listed in CSS_EXEMPT below.
import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, relative, resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const src = resolve(root, 'src');

// Not app chrome: the OBS browser source (the streamer picks its
// background), and the floating chat overlay window (its own opacity slider).
const SKIP_DIRS = new Set(['overlay']);
const SKIP_FILES = new Set(['ChatOverlayWindow.tsx']);

// CSS rules that frost without riding the slider, on purpose.
const CSS_EXEMPT: Array<[RegExp, string]> = [
  [/\.atm-/, 'atmospheres belong to a member, not to the interface'],
  [/\.chat-fullscreen-overlay/, 'the full-screen chat column has its own opacity setting'],
  [/\.sn-chat-overlay|\.sn-overlay-/, 'the floating chat overlay window has its own opacity slider'],
  [/^\.backdrop-blur-ultra$/, 'a blur utility; the element using it carries the fill'],
  [/\.plyr__controls/, 'the player control bar is a gradient scrim over the video'],
  [/^\.ghost-card$/, 'a placeholder tile in the Home grid; it never floats'],
  [/^\.update-pill$/, 'a tint inside the title bar, which carries the surface'],
];

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (!SKIP_DIRS.has(name)) walk(p, out);
    } else {
      out.push(p);
    }
  }
  return out;
}
const files = walk(src);
const cache = new Map<string, string>();
const read = (p: string) => {
  let text = cache.get(p);
  if (text === undefined) {
    text = readFileSync(p, 'utf8');
    cache.set(p, text);
  }
  return text;
};
const rel = (p: string) => relative(root, p).replace(/\\/g, '/');
const lineAt = (text: string, index: number) => text.slice(0, index).split('\n').length;

const TRANSLUCENT = new RegExp(
  [
    String.raw`rgba?\([^)]*[,/]\s*0?\.\d+\s*\)`,
    String.raw`hsla?\([^)]*[,/]\s*0?\.\d+\s*\)`,
    String.raw`#[0-9a-fA-F]{8}\b`,
    String.raw`color-mix\([^;]*transparent`,
    String.raw`var\(--color-background-secondary\)`,
    String.raw`\btransparent\b`,
  ].join('|'),
);

/** Opaque, or rides the slider. */
function awareValue(value: string, localVars: Record<string, string> = {}): boolean {
  if (/glass-strength|--glass-under|--veil/.test(value)) return true;
  // A value built from a custom property the same rule defines (.sn-popover's
  // --sn-popover-tint) rides the slider if that property does.
  for (const m of value.matchAll(/var\((--[\w-]+)/g)) {
    if (/glass-strength/.test(localVars[m[1]] ?? '')) return true;
  }
  return !TRANSLUCENT.test(value);
}

// ---- CSS ----------------------------------------------------------------
interface CssRule {
  selector: string;
  body: string;
  line: number;
}
function cssRules(text: string): CssRule[] {
  const clean = text.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
  const out: CssRule[] = [];
  const parse = (start: number, end: number) => {
    let j = start;
    while (j < end) {
      const open = clean.indexOf('{', j);
      if (open < 0 || open >= end) break;
      const selector = clean.slice(j, open).trim();
      let depth = 1;
      let k = open + 1;
      while (k < end && depth) {
        if (clean[k] === '{') depth++;
        else if (clean[k] === '}') depth--;
        k++;
      }
      if (/^@(media|supports|layer)/.test(selector)) parse(open + 1, k - 1);
      else if (!selector.startsWith('@')) out.push({ selector, body: clean.slice(open + 1, k - 1), line: lineAt(clean, open) });
      j = k;
    }
  };
  parse(0, clean.length);
  return out;
}
function decl(body: string, prop: string): string | null {
  const all = [...body.matchAll(new RegExp(String.raw`(?:^|;)\s*` + prop + String.raw`\s*:\s*([^;]+)`, 'g'))];
  return all.length ? all[all.length - 1][1].replace(/\s+/g, ' ').trim() : null;
}

// ---- TSX ----------------------------------------------------------------
const GLASS_CLASS =
  /(?<![\w-])(glass-panel|glass-modal|glass-flyout|glass-badge|glass-button[\w-]*|liquid-glass-panel|sn-popover|glass-input|sn-glass-veil|user-profile-card)(?![\w-])/;
const POSITIONED = /(?<![\w:-])(fixed|absolute|sticky)(?![\w-])/;
const BLUR = /(?<![\w:-])backdrop-blur(?:-[a-z0-9]+|-\[[^\]]+\])?(?![\w-])/;
const SCRIM = /(?<![\w-])inset-0(?![\w-])/;
// A 1-4px line or bar is a mark, not a surface. pointer-events-none alone is
// not enough: a tooltip ignores the pointer and still carries text, so it only
// marks a decoration when nothing frosts it.
const THIN = /(?<![\w:-])([hw]-px|[hw]-0\.5|[hw]-1|[hw]-\[[1-4]px\])(?![\w-])/;
const INERT = /(?<![\w:-])pointer-events-none(?![\w-])/;
// Base (unprefixed or breakpoint-prefixed) background fills.
const BG = /(?<![\w:-])((?:(?:sm|md|lg|xl|2xl|portrait|landscape):)*)(!?)bg-(\[[^\]]+\]|[a-zA-Z]+(?:-[a-zA-Z0-9]+)*)(?:\/(\[[^\]]+\]|\d+))?/g;
const NOT_A_FILL = /^(clip-|none$|cover$|contain$|center$|no-repeat$|fixed$|left$|right$|top$|bottom$|repeat|origin-|blend-|gradient|opacity)/;

interface Fill {
  name: string;
  alpha: string | undefined;
  token: string;
}
function baseFills(expr: string): Fill[] {
  const out: Fill[] = [];
  for (const m of expr.matchAll(BG)) {
    const [token, , , name, alpha] = m;
    if (NOT_A_FILL.test(name)) continue;
    out.push({ name, alpha, token });
  }
  return out;
}
function fillAware(f: Fill): boolean {
  if (f.name === 'glass' || f.name.startsWith('glass-')) return true;
  if (f.name.startsWith('[')) {
    const v = f.name.slice(1, -1);
    return !f.alpha && (/^#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/.test(v) || /glass-strength/.test(v));
  }
  if (f.alpha) return false;
  return !['secondary', 'transparent', 'current', 'inherit'].includes(f.name);
}

interface Element {
  line: number;
  expr: string;
  inline: string | null;
  /** Positioned by an inline `position`, not a class. */
  inlinePositioned: boolean;
  exempt: boolean;
}

/** An object-literal property's value, up to its top-level comma or brace. */
function propValue(body: string, prop: RegExp): string | null {
  const m = prop.exec(body);
  if (!m) return null;
  let depth = 0;
  let k = m.index + m[0].length;
  const start = k;
  for (; k < body.length; k++) {
    const c = body[k];
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') {
      if (depth === 0) break;
      depth--;
    } else if ((c === ',' || c === '\n') && depth === 0) break;
  }
  return body.slice(start, k).trim();
}
function elements(text: string): Element[] {
  const consts: Record<string, string> = {};
  for (const m of text.matchAll(/const\s+(\w+)\s*(?::[^=\n]+)?=\s*\{/g)) {
    let depth = 1;
    let k = m.index! + m[0].length;
    while (k < text.length && depth) {
      if (text[k] === '{') depth++;
      else if (text[k] === '}') depth--;
      k++;
    }
    consts[m[1]] = text.slice(m.index! + m[0].length, k - 1);
  }
  // Class strings held in constants (const CHROME = 'rounded-md bg-black/80 ...'),
  // so a className built from them is read with their classes.
  const classConsts: Record<string, string> = {};
  for (const m of text.matchAll(/const\s+(\w+)\s*(?::\s*string\s*)?=\s*(['"`])([^'"`]*?)\2/g)) {
    if (/(^|\s)(bg-|backdrop-blur|absolute|fixed|sticky|glass-)/.test(m[3])) classConsts[m[1]] = m[3];
  }
  const withConsts = (expr: string) =>
    expr.replace(/\b([A-Za-z_]\w*)\b/g, (id) => (id in classConsts ? `${id} ${classConsts[id]}` : id));
  const lines = text.split('\n');
  const out: Element[] = [];
  for (const m of text.matchAll(/className=/g)) {
    const i = m.index! + m[0].length;
    let expr: string;
    let after: number;
    if (text[i] === '"' || text[i] === "'") {
      const j = text.indexOf(text[i], i + 1);
      expr = text.slice(i + 1, j);
      after = j + 1;
    } else if (text[i] === '{') {
      let depth = 1;
      let k = i + 1;
      while (k < text.length && depth) {
        if (text[k] === '{') depth++;
        else if (text[k] === '}') depth--;
        k++;
      }
      expr = text.slice(i + 1, k - 1);
      after = k;
    } else {
      continue;
    }
    const tagStart = text.lastIndexOf('<', m.index!);
    const tagEnd = text.indexOf('>', after);
    const region = text.slice(tagStart, m.index!) + ' ' + text.slice(after, tagEnd < 0 ? after + 1200 : tagEnd);
    let inline: string | null = null;
    let inlinePositioned = false;
    const sm = /style=\{/.exec(region);
    if (sm) {
      let depth = 1;
      let k = sm.index + sm[0].length;
      while (k < region.length && depth) {
        if (region[k] === '{') depth++;
        else if (region[k] === '}') depth--;
        k++;
      }
      let body = region.slice(sm.index + sm[0].length, k - 1).trim();
      if (/^\w+$/.test(body)) body = consts[body] ?? '';
      inline = propValue(body, /\bbackground(?:Color)?\s*:\s*/);
      inlinePositioned = /\bposition\s*:\s*['"](fixed|absolute|sticky)['"]/.test(body);
    }
    const line = lineAt(text, m.index!);
    // The four lines above the element's opening tag, through its className.
    const tagLine = lineAt(text, Math.max(0, tagStart));
    const above = lines.slice(Math.max(0, tagLine - 5), line).join('\n');
    out.push({ line, expr: withConsts(expr), inline, inlinePositioned, exempt: /glass-exempt:/.test(above) });
  }
  return out;
}

const parsedCache = new Map<string, Element[]>();
const parsed = (file: string) => {
  let els = parsedCache.get(file);
  if (!els) {
    els = elements(read(file));
    parsedCache.set(file, els);
  }
  return els;
};

describe('Glassiness 0% leaves no see-through surface', () => {
  it('every frosted CSS rule rides the slider', () => {
    const bad: string[] = [];
    for (const file of files.filter((f) => f.endsWith('.css'))) {
      for (const rule of cssRules(read(file))) {
        const blur = decl(rule.body, 'backdrop-filter') ?? decl(rule.body, '-webkit-backdrop-filter');
        if (!blur || blur.startsWith('none')) continue;
        const selector = rule.selector.replace(/\s+/g, ' ');
        if (CSS_EXEMPT.some(([re]) => re.test(selector))) continue;
        const localVars: Record<string, string> = {};
        for (const m of rule.body.matchAll(/(--[\w-]+)\s*:\s*([^;]+)/g)) localVars[m[1]] = m[2];
        const fill = decl(rule.body, 'background-color') ?? decl(rule.body, 'background');
        if (fill && awareValue(fill, localVars)) continue;
        bad.push(`${rel(file)}:${rule.line} ${selector} { background: ${fill ?? '(none)'} }`);
      }
    }
    expect(bad, 'Frosted rules whose fill stays translucent at Glassiness 0%. Ramp the alpha: ' +
      'color-mix(in srgb, <colour> calc(A% + (1 - var(--glass-strength, 1)) * (100% - A%)), transparent)').toEqual([]);
  });

  it('every positioned or frosted surface rides the slider', () => {
    const bad: string[] = [];
    for (const file of files.filter((f) => /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f))) {
      if (SKIP_FILES.has(file.split(/[\\/]/).pop()!)) continue;
      for (const el of parsed(file)) {
        if (el.exempt || GLASS_CLASS.test(el.expr)) continue;
        const positioned = POSITIONED.test(el.expr) || el.inlinePositioned;
        const frosted = BLUR.test(el.expr);
        if (!positioned && !frosted) continue;
        // A backdrop blur asks to show what is behind it, so a frosted element
        // is a surface wherever it sits (a tooltip's frosted chrome sits in
        // flow inside a clear, positioned wrapper). Scrims excepted.
        if (positioned && SCRIM.test(el.expr)) continue;
        if (!frosted && (THIN.test(el.expr) || INERT.test(el.expr))) continue;
        const fills = baseFills(el.expr);
        const aware = el.inline !== null ? awareValue(el.inline) : fills.some(fillAware);
        const translucent = el.inline !== null ? !aware : fills.length > 0 && !aware;
        // Frosted with no fill at all is see-through at 0%; positioned with no
        // fill is layout, not a surface.
        if (aware || (!translucent && !(frosted && fills.length === 0 && el.inline === null))) continue;
        const what = el.inline ?? (fills.map((f) => f.token).join(' ') || '(no fill)');
        bad.push(`${rel(file)}:${el.line} ${what}`);
      }
    }
    expect(bad, 'Surfaces that stay see-through at Glassiness 0%. Use a glass colour ' +
      '(bg-glass-ink/90, bg-glass-raised/95, bg-glass-base/95; /0 is clear at full glass), ' +
      'or add a "glass-exempt: <reason>" comment above a deliberate one.').toEqual([]);
  });

  it('a glass element keeps its base under a hand-written fill', () => {
    const bad: string[] = [];
    for (const file of files.filter((f) => /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f))) {
      if (SKIP_FILES.has(file.split(/[\\/]/).pop()!)) continue;
      for (const el of parsed(file)) {
        if (el.exempt || !GLASS_CLASS.test(el.expr)) continue;
        if (el.inline !== null && !awareValue(el.inline)) {
          bad.push(`${rel(file)}:${el.line} style background ${el.inline}`);
        }
        // Arbitrary colours skip the Tailwind colour functions, so they never
        // mix over --glass-under.
        for (const m of el.expr.matchAll(/(?<![\w-])(?:[\w-]+:)*!?bg-\[([^\]]+)\](?:\/(\[[^\]]+\]|\d+))?/g)) {
          if (m[2] || TRANSLUCENT.test(m[1])) bad.push(`${rel(file)}:${el.line} ${m[0]}`);
        }
      }
    }
    expect(bad, 'An inline or arbitrary fill on a glass element replaces its solid base at 0%. ' +
      'Mix it over the base: color-mix(in srgb, <tint> <alpha>, var(--glass-under, transparent)).').toEqual([]);
  });

  it('background colours mix over the glass base, and glass colours ramp to opaque', () => {
    type ColorFn = (o: { opacityValue?: string }) => string;
    type Colors = Record<string, ColorFn> & { glass: Record<string, ColorFn> };
    const config = createRequire(import.meta.url)('../../tailwind.config.js') as {
      theme: { extend: { backgroundColor: Colors; colors: Colors } };
    };
    const bg = config.theme.extend.backgroundColor;
    expect(bg.white({ opacityValue: '0.1' })).toContain('var(--glass-under, transparent)');
    expect(bg.accent({ opacityValue: '0.2' })).toContain('var(--glass-under, transparent)');
    expect(bg.glass.ink({ opacityValue: '0.9' })).toContain('var(--glass-strength, 1)');
    // Text, border, ring and shadow colours stay as they were: an opaque base
    // under something painted over other content would show as a halo.
    expect(config.theme.extend.colors.accent({ opacityValue: '0.2' })).not.toContain('--glass-under');
  });
});

// WebView2 paints some backdrop-filter combinations wrong while a plain
// Chromium paints them right, so a lab can only ever prove one broken. These
// read the source for the two combinations already seen to fail in the app.
describe('Backdrop filters stay out of combinations WebView2 mispaints', () => {
  const css = files.filter((f) => f.endsWith('.css'));
  const rules = css.flatMap((f) => cssRules(read(f)).map((r) => ({ ...r, file: f })));
  const frosted = (body: string) => {
    const v = decl(body, 'backdrop-filter') ?? decl(body, '-webkit-backdrop-filter');
    return v !== null && !v.startsWith('none');
  };
  const squircle = (body: string) => {
    const v = decl(body, 'corner-shape');
    return v !== null && !/^(inherit|round|none)$/.test(v);
  };

  it('no squircle corner on a frosted surface', () => {
    // A superellipse corner on an element that also carries a backdrop filter
    // makes the content inside it fail to paint.
    const bad: string[] = [];
    // A class whose own rule turns the filter off is safe beside glass classes.
    const unfrosts = new Set<string>();
    for (const r of rules) {
      const v = decl(r.body, 'backdrop-filter');
      if (v !== null && v.startsWith('none')) {
        for (const m of r.selector.matchAll(/\.([\w-]+)/g)) unfrosts.add(m[1]);
      }
    }
    const squircleClasses = new Set<string>();
    for (const r of rules) {
      if (!squircle(r.body)) continue;
      if (frosted(r.body)) bad.push(`${rel(r.file)}:${r.line} ${r.selector.replace(/\s+/g, ' ')}`);
      for (const sel of r.selector.split(',')) {
        const m = /^\s*\.([\w-]+)\s*$/.exec(sel);
        if (m && !unfrosts.has(m[1])) squircleClasses.add(m[1]);
      }
    }
    for (const file of files.filter((f) => f.endsWith('.tsx') && !/\.test\.tsx$/.test(f))) {
      for (const el of parsed(file)) {
        const has = (c: string) => new RegExp(String.raw`(?<![\w-])${c}(?![\w-])`).test(el.expr);
        const shaped = [...squircleClasses].find(has);
        if (!shaped) continue;
        if ((GLASS_CLASS.test(el.expr) || BLUR.test(el.expr)) && !has('no-live-blur')) {
          bad.push(`${rel(file)}:${el.line} .${shaped} on a frosted element`);
        }
      }
    }
    expect(bad, 'corner-shape: squircle and a backdrop filter on one element. Drop one of them ' +
      '(a class that needs both can switch the filter off in its own rule)').toEqual([]);
  });

  it('a light blur inside a heavy one is switched off', () => {
    // globals.css strips light glass blurs inside heavily blurred surfaces (one
    // blur per stack). A class added with a backdrop filter has to join the
    // list its radius belongs in, or it quietly buys a layer that draws nothing.
    const stack = rules.find((r) => /:is\([^)]*liquid-glass-panel[\s\S]*\)\s*:is\(/.test(r.selector));
    expect(stack, 'the one-blur-per-stack rule moved; point this test at it').toBeTruthy();
    const [heavyList, lightList] = [...stack!.selector.matchAll(/:is\(([^)]*)\)/g)].map((m) => m[1]);
    const listed = (list: string, c: string) => new RegExp(`\\.${c}(?![\\w-])`).test(list);
    // Light classes that stay out on purpose.
    const LIGHT_EXEMPT: Record<string, string> = {
      'glass-badge': 'a count chip over a thumbnail blurs real picture content',
      'card-chip': 'a chip over card art blurs real picture content',
      'atm-frost': 'atmospheres belong to a member, not to the interface',
      'chrome-glaze': 'title-bar chrome, never nested in a heavy surface',
      'chrome-glaze--dock': 'title-bar chrome, never nested in a heavy surface',
      'chat-fullscreen-overlay': 'the full-screen chat column is a surface of its own',
      'user-profile-card': 'a floating card, never nested in a heavy surface',
      'stats-hud': 'sits over the video, never nested in a heavy surface',
      'ghost-card': 'a placeholder tile in the Home grid',
      'update-pill': 'a tint inside the title bar',
      'hype-train-track': 'sits over the chat, never nested in a heavy surface',
      'drops-preview-card-right': 'a floating preview card',
      'sn-chapter-tip': 'a timeline tooltip over the video',
      'sn-timeline__tip': 'a timeline tooltip over the video',
      'glass-flyout': 'heavy in practice (it runs at 42px live), so it is a root',
    };
    const radius = (body: string) => {
      const v = decl(body, 'backdrop-filter') ?? '';
      const m = /blur\(\s*(?:calc\(\s*(?:var\([^,]+,\s*)?)?([\d.]+)px/.exec(v);
      return m ? Number(m[1]) : null;
    };
    const bad: string[] = [];
    for (const r of rules) {
      if (!frosted(r.body)) continue;
      const px = radius(r.body);
      if (px === null) continue;
      for (const sel of r.selector.split(',')) {
        const c = /^\s*\.([\w-]+)\s*$/.exec(sel)?.[1];
        if (!c) continue;
        if (px >= 24 && !listed(heavyList, c)) bad.push(`.${c} (${px}px) belongs in the heavy list`);
        if (px <= 16 && !listed(lightList, c) && !listed(heavyList, c) && !(c in LIGHT_EXEMPT)) {
          bad.push(`.${c} (${px}px) belongs in the light list, or LIGHT_EXEMPT with a reason`);
        }
      }
    }
    expect([...new Set(bad)], 'Backdrop-filtered classes missing from the one-blur-per-stack rule in globals.css').toEqual([]);
  });

  it('chat link previews carry no live blur', () => {
    // Chat rows use content-visibility: auto, and a backdrop filter inside one
    // leaves stale copies of the card behind as the list scrolls.
    const file = files.find((f) => rel(f) === 'src/components/chat/LinkPreviewCard.tsx');
    expect(file, 'LinkPreviewCard.tsx moved; point this test at it').toBeTruthy();
    const bad = parsed(file!)
      .filter((el) => (GLASS_CLASS.test(el.expr) || BLUR.test(el.expr)) && !/(?<![\w-])no-live-blur(?![\w-])/.test(el.expr))
      .map((el) => `${rel(file!)}:${el.line}`);
    expect(bad, 'Frosted link-preview elements without no-live-blur').toEqual([]);
  });
});
