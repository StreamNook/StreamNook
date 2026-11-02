// Run with: npm test
//
// Motion Off and the cheap-draw modes stop CSS animation with one rule each
// (globals.css: the Motion preferences block and Cheap draw), so every
// animation in the app carries a decision here. A capped loop snaps to its
// un-animated style, which looks wrong for some (a sweep band, a glint arc); a
// loader must keep moving; a one-shot cue whose meaning is in its middle
// frames must not complete at once. A new animation fails this test until it
// is listed with its decision, and a listed one that is gone fails as stale.
import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, relative, resolve, sep } from 'node:path';
import postcss, { type AtRule, type Root, type Rule } from 'postcss';

const root = resolve(__dirname, '..', '..');
const src = resolve(root, 'src');

type Decision =
  // A loop whose un-animated style is its designed still look.
  | 'rests-ok'
  // A loop with a named rest under html[data-sn-low-power] (written for `cover`).
  | 'rest'
  // Keeps moving: a loader, or the offline room's drift (`cover` is exempt).
  | 'keeps'
  // Chat content (emote modifiers): exempt, stopped by Freeze emotes instead.
  | 'content'
  // A one-shot cue whose meaning is in its middle frames (`cover` is exempt).
  | 'cue'
  // A finite animation that may complete at once in Performance mode.
  | 'instant-ok';

interface Row {
  d: Decision;
  // keeps, content, cue: the cap's exempt selector that covers it.
  // rest: the selector its rest is written for under html[data-sn-low-power].
  cover?: string;
  // instant-ok whose first and last frames match: why it may still complete at once.
  why?: string;
}

const LOOP_DECISIONS = new Set<Decision>(['rests-ok', 'rest', 'keeps', 'content']);
const ONCE_DECISIONS = new Set<Decision>(['keeps', 'content', 'cue', 'instant-ok']);
const EXEMPT_DECISIONS = new Set<Decision>(['keeps', 'content', 'cue']);

const FFZ = '[class*="sn-ffz-anim-"]';
const BTTV = '[class*="sn-bttv-anim-"]';

// Every rule in a stylesheet under src that starts an animation, keyed by
// file and selector list.
const CSS_ROWS: Record<string, Row> = {
  'components/MajorCologneChrome.css|.cologne-wash': { d: 'rest', cover: '.cologne-wash' },
  'components/chat/listeningStrip.css|.sn-listen-dots > i': { d: 'keeps', cover: '.sn-listen-dots > i' },
  'mobile/mobile.css|.sn-atm-breathe-1': { d: 'rest', cover: '.sn-atm-breathe-1' },
  'mobile/mobile.css|.sn-atm-breathe-2': { d: 'rest', cover: '.sn-atm-breathe-2' },
  'mobile/mobile.css|.sn-mobile .sn-chat-reveal': { d: 'instant-ok' },
  'mobile/mobile.css|.sn-mobile .chat-message-row[data-sn-flash]::after': {
    d: 'cue',
    cover: '.chat-message-row[data-sn-flash]',
  },
  'styles/globals.css|.update-pill': { d: 'rest', cover: '.update-pill' },
  'styles/globals.css|.sn-bulb-rainbow stop': { d: 'rests-ok' },
  'styles/globals.css|.sn-aurora-1': { d: 'rest', cover: '.sn-aurora-1' },
  'styles/globals.css|.sn-aurora-2': { d: 'rest', cover: '.sn-aurora-2' },
  'styles/globals.css|.animate-heart-drop': { d: 'instant-ok' },
  'styles/globals.css|.sn-giftbomb-border::before': { d: 'rests-ok' },
  'styles/globals.css|.settings-row-found::after': { d: 'cue', cover: '.settings-row-found' },
  'styles/globals.css|html[data-motion="reduced"] .animate-scale-in': { d: 'instant-ok' },
  'styles/globals.css|.sub-progress-fill': { d: 'instant-ok' },
  'styles/globals.css|.animate-highlight-flash': { d: 'cue', cover: '.animate-highlight-flash' },
  'styles/globals.css|.animate-mention-flash': { d: 'cue', cover: '.animate-mention-flash' },
  'styles/globals.css|.animate-reply-flash': { d: 'cue', cover: '.animate-reply-flash' },
  'styles/globals.css|.animate-phrase-highlight': { d: 'cue', cover: '.animate-phrase-highlight' },
  "styles/globals.css|[data-entrance='fade'] > .chat-message-row:not(.is-backfill):nth-last-child(-n + 3)": {
    d: 'instant-ok',
  },
  "styles/globals.css|[data-entrance='slide'] > .chat-message-row:not(.is-backfill):nth-last-child(-n + 3)": {
    d: 'instant-ok',
  },
  "styles/globals.css|[data-entrance='rise'] > .chat-message-row:not(.is-backfill):nth-last-child(-n + 3)": {
    d: 'instant-ok',
  },
  'styles/globals.css|.sn-tabs-unfolded': { d: 'instant-ok' },
  'styles/globals.css|.sn-ft-anim-ring.sn-ft-t-sheen::after': { d: 'instant-ok' },
  'styles/globals.css|.sn-ft-anim-ring.sn-ft-t-sheen.sn-ft-loop::after': {
    d: 'rest',
    cover: '.sn-ft-anim-ring.sn-ft-loop::after',
  },
  'styles/globals.css|.sn-ft-anim-bar.sn-ft-t-sheen::after, .sn-ft-anim-bar.sn-ft-t-chase::after': {
    d: 'instant-ok',
  },
  'styles/globals.css|.sn-ft-anim-bar.sn-ft-t-sheen.sn-ft-loop::after, .sn-ft-anim-bar.sn-ft-t-chase.sn-ft-loop::after': {
    d: 'rest',
    cover: '.sn-ft-anim-bar.sn-ft-loop::after',
  },
  'styles/globals.css|.sn-ft-anim-ring.sn-ft-t-pulse::after, .sn-ft-anim-bar.sn-ft-t-pulse::after': {
    d: 'instant-ok',
    why: 'a decorative glint over a highlighted row; Performance mode drops decoration',
  },
  'styles/globals.css|.sn-ft-anim-ring.sn-ft-t-pulse.sn-ft-loop::after, .sn-ft-anim-bar.sn-ft-t-pulse.sn-ft-loop::after': {
    d: 'rest',
    cover: '.sn-ft-anim-ring.sn-ft-loop::after',
  },
  'styles/globals.css|.sn-ft-anim-ring.sn-ft-t-chase::after': { d: 'instant-ok' },
  'styles/globals.css|.sn-ft-anim-ring.sn-ft-t-chase.sn-ft-loop::after': {
    d: 'rest',
    cover: '.sn-ft-anim-ring.sn-ft-loop::after',
  },
  'styles/globals.css|.iridescent-title': { d: 'rests-ok' },
  'styles/globals.css|.iridescent-border::after': { d: 'rests-ok' },
  'styles/globals.css|.hype-train-header': { d: 'rests-ok' },
  'styles/globals.css|.hype-train-progress-rainbow::before, .hype-train-progress-golden::before': {
    d: 'rest',
    cover: '.hype-train-progress-rainbow::before',
  },
  'styles/globals.css|.hype-train-progress-bar': { d: 'rests-ok' },
  'styles/globals.css|.hype-train-level-up': {
    d: 'instant-ok',
    why: 'a rare bump on the hype train badge; the new level number carries the news',
  },
  'styles/globals.css|.animate-hype-flash': { d: 'instant-ok' },
  'styles/globals.css|.animate-hype-marquee': { d: 'cue', cover: '.animate-hype-marquee' },
  'styles/globals.css|.hype-train-fill-wave': { d: 'rest', cover: '.hype-train-fill-wave' },
  'styles/globals.css|.animate-fly-up-fade': { d: 'instant-ok' },
  'styles/globals.css|.animate-multinook-suck-up': { d: 'instant-ok' },
  'styles/globals.css|.animate-multinook-materialize': { d: 'instant-ok' },
  'styles/globals.css|.ghost-card': { d: 'keeps', cover: '.ghost-card' },
  'styles/globals.css|.animate-ghost-label': { d: 'instant-ok' },
  'styles/globals.css|.animate-heart-break': { d: 'instant-ok' },
  'styles/globals.css|.animate-penrose-rotate': { d: 'keeps', cover: '.animate-penrose-rotate' },
  'styles/globals.css|.penrose-march .pm-cube': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|html[data-motion="off"] .penrose-march .pm-cube': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march .pm-flow-a': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march .pm-flow-b': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march .pm-flow-c': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march .pm-flow-c.pm-phase-a': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march .pm-flow-c.pm-phase-b': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march--layered .pm-flow-a': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march--layered .pm-flow-b': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march--layered .pm-flow-c': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march--layered .pm-flow-c.pm-phase-a': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.penrose-march--layered .pm-flow-c.pm-phase-b': { d: 'keeps', cover: '.pm-cube' },
  'styles/globals.css|.sn-mod-dwell': { d: 'cue', cover: '.sn-mod-dwell' },
  'styles/globals.css|.sn-mod-chip-label': { d: 'instant-ok' },
  'styles/globals.css|.sn-mod-track': { d: 'instant-ok' },
  'styles/globals.css|.sn-mod-rail': { d: 'instant-ok' },
  'styles/globals.css|.sn-mod-cap-dwell': { d: 'cue', cover: '.sn-mod-cap-dwell' },
  "styles/globals.css|[data-line-show='hover']:not([data-line-aim]) .chat-message-row:hover .sn-mod-line, [data-line-show='hover'][data-line-aim] .chat-message-row[data-line-open] .sn-mod-line": {
    d: 'instant-ok',
  },
  'styles/globals.css|.shimmer-text': { d: 'rests-ok' },
  'styles/globals.css|.shimmer-border::before': { d: 'rests-ok' },
  'styles/globals.css|.green-drop-animation': { d: 'rests-ok' },
  'styles/globals.css|.blue-drop-animation': { d: 'rests-ok' },
  'styles/globals.css|.rainbow-drop-animation': { d: 'rests-ok' },
  'styles/globals.css|.rainbow-puddle': { d: 'rests-ok' },
  'styles/globals.css|.rainbow-drop-icon': { d: 'rests-ok' },
  'styles/globals.css|.automation-pulse': { d: 'rest', cover: '.automation-pulse' },
  'styles/globals.css|.automation-pulse-shift': { d: 'rest', cover: '.automation-pulse-shift' },
  'styles/globals.css|.rainbow-puddle-icon': { d: 'rests-ok' },
  'styles/globals.css|.notification-enter': { d: 'instant-ok' },
  'styles/globals.css|.notification-exit': { d: 'instant-ok' },
  'styles/globals.css|.island-expand': {
    d: 'instant-ok',
    why: "the island's own resize already shows it opening",
  },
  'styles/globals.css|.unread-pulse': { d: 'rests-ok' },
  'styles/globals.css|.live-glow': { d: 'rests-ok' },
  'styles/globals.css|.whisper-highlight': { d: 'cue', cover: '.whisper-highlight' },
  'styles/globals.css|.group:hover .card-chip-dot': { d: 'rests-ok' },
  'styles/globals.css|.card-chip-shine > span': { d: 'rest', cover: '.card-chip-shine > span' },
  'styles/globals.css|.missing-now-pulse': { d: 'rests-ok' },
  'styles/globals.css|.animate-progress-shimmer': { d: 'rests-ok' },
  'styles/globals.css|.animate-slide-in-right': { d: 'instant-ok' },
  'styles/globals.css|.animate-slide-out-right': { d: 'instant-ok' },
  'styles/globals.css|.mining-card-border::before': { d: 'rests-ok' },
  'styles/globals.css|.automation-shimmer-overlay::after': {
    d: 'rest',
    cover: '.automation-shimmer-overlay::after',
  },
  'styles/globals.css|.automation-progress-bar': { d: 'rests-ok' },
  'styles/globals.css|.mining-card-border::after': { d: 'rests-ok' },
  'styles/globals.css|.drops-progress-inline': { d: 'rest', cover: '.drops-progress-inline' },
  'styles/globals.css|.drops-preview-card-right': { d: 'instant-ok' },
  'styles/globals.css|.animate-panel-slide-down': { d: 'instant-ok' },
  'styles/globals.css|.animate-panel-slide-up': { d: 'instant-ok' },
  'styles/globals.css|.sn-ethereal-text': { d: 'rests-ok' },
  'styles/globals.css|.chest-attention': { d: 'rests-ok' },
  'styles/globals.css|.sn-ffz-anim-rainbow': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-shake': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-jam': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-bounce': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-spin': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-slide': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-appear': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-ffz-anim-leave': { d: 'content', cover: FFZ },
  'styles/globals.css|.sn-bttv-anim-party': { d: 'content', cover: BTTV },
  'styles/globals.css|.sn-bttv-anim-shake': { d: 'content', cover: BTTV },
  'styles/globals.css|.offline-bounce-x': { d: 'keeps', cover: '.offline-bounce-x' },
  'styles/globals.css|.offline-bounce-y': { d: 'keeps', cover: '.offline-bounce-y' },
  'styles/globals.css|html[data-motion] div.offline-bounce-x': { d: 'keeps', cover: '.offline-bounce-x' },
  'styles/globals.css|html[data-motion] div.offline-bounce-y': { d: 'keeps', cover: '.offline-bounce-y' },
  'styles/globals.css|.sn-tbt-bar': { d: 'rest', cover: '.sn-tbt-bar' },
  'styles/globals.css|.sn-tbt-pointer': { d: 'rest', cover: '.sn-tbt-pointer' },
};

// tailwind.config.js `theme.extend.animation`.
const TAILWIND_ROWS: Record<string, Row> = {
  'fade-in': { d: 'instant-ok' },
  'scale-in': { d: 'instant-ok' },
  shimmer: { d: 'rests-ok' },
  droplet: { d: 'rests-ok' },
  splash: { d: 'instant-ok' },
  ripple: { d: 'rests-ok' },
};

// Exempt selectors no stylesheet row names: the loaders that live in TSX.
const TSX_EXEMPT = ['.animate-spin', '.animate-pulse:not(.sn-decor)', '.sn-loader', '.sn-loader *'];

// Motion Off keeps only the loaders moving.
const OFF_EXEMPT = ['.animate-spin', '.animate-penrose-rotate', '.sn-loader', '.sn-loader *'];

// Files whose unmarked `animate-pulse` is a loading skeleton or a busy state.
// A decorative pulse anywhere carries `sn-decor` beside it.
const PULSE_LOADER_FILES = new Set([
  'components/badge/EvolutionPicker.tsx',
  'components/badge/MissingNowStrip.tsx',
  'components/ChannelPointsMenu.tsx',
  'components/Home.tsx',
  'components/modroom/ModRoomPane.tsx',
  'components/SocialLinks.tsx',
  'components/StreamerAboutPanel.tsx',
  // The whispers button's dot, shown only while an import runs.
  'components/TitleBar.tsx',
  'components/UserProfileCard.tsx',
  'components/WhispersWidget.tsx',
  'mobile/rewards/MobileEvolution.tsx',
  'mobile/rewards/MobileMissingNow.tsx',
  'mobile/screens/MobileWhatsNew.tsx',
  'mobile/ui/SkeletonCards.tsx',
]);

// Arbitrary `animate-[...]` classes, by file.
const ARBITRARY_ANIMATE: Record<string, Decision> = {
  // The live dot pulses on hover; it rests as a still dot.
  'components/Sidebar.tsx': 'rests-ok',
};

// Inline styles and injected <style> strings that loop forever, by file.
const INLINE_LOOPS: Record<string, Decision> = {
  // The top three rows' shimmer; a still gradient at rest.
  'components/ChannelPointsLeaderboard.tsx': 'rests-ok',
  // The emote-unlock reveal's spinning glow; decoration.
  'components/ChannelPointsMenu.tsx': 'rests-ok',
  // "Waiting for you to authorize": a loader, marked `sn-loader`.
  'components/DeviceLoginOverlay.tsx': 'keeps',
  // The overlay's copies of the glint and atmosphere loops, rested by class.
  'components/overlay/OverlayChat.tsx': 'rests-ok',
};

// framer-motion loops, which the CSS caps cannot reach.
const FRAMER_LOOPS: Record<string, 'held' | 'words'> = {
  // `holdStill` swaps both `animate` and `transition`.
  'components/profile/MemberProfileBackdrop.tsx': 'held',
  'components/profile/MemberProfileHero.tsx': 'held',
  // Below Full motion the swipe hint is words, not motion.
  'components/SetupWizard.tsx': 'words',
};

const QUIET = 'html[data-sn-low-power] *:not(';
const PERF = 'html[data-sn-perf] *:not(';
const OFF = 'html[data-motion="off"] *:not(';
const CHEAP_ATTRS = ['[data-sn-low-power]', '[data-sn-perf]', '[data-sn-still-chat]'];

const norm = (s: string) =>
  s.replace(/\s+/g, ' ').replace(/\(\s+/g, '(').replace(/\s+\)/g, ')').trim();
const rel = (p: string) => relative(src, p).split(sep).join('/');

function walk(dir: string, keep: (name: string) => boolean, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, keep, out);
    else if (keep(name)) out.push(p);
  }
  return out;
}

const sheets: { file: string; root: Root }[] = walk(src, (n) => n.endsWith('.css')).map((p) => ({
  file: rel(p),
  root: postcss.parse(readFileSync(p, 'utf8'), { from: p }),
}));
const globals = sheets.find((s) => s.file === 'styles/globals.css')!.root;

// First and last frames of every keyframes rule, as sorted `prop:value` lists.
type Frames = { from: string | null; to: string | null };
const frames = new Map<string, Frames>();
const body = (pairs: [string, string][]) =>
  pairs
    .map(([k, v]) => `${k}:${norm(v)}`)
    .sort()
    .join(';');
for (const { root: sheet } of sheets) {
  sheet.walkAtRules(/keyframes$/, (at) => {
    const f: Frames = { from: null, to: null };
    for (const node of at.nodes ?? []) {
      if (node.type !== 'rule') continue;
      const stops = node.selectors.map((s) => s.trim());
      const decls: [string, string][] = [];
      for (const d of node.nodes) if (d.type === 'decl') decls.push([d.prop, d.value]);
      if (stops.includes('from') || stops.includes('0%')) f.from = body(decls);
      if (stops.includes('to') || stops.includes('100%')) f.to = body(decls);
    }
    frames.set(at.params.trim(), f);
  });
}
const tailwind = createRequire(import.meta.url)('../../tailwind.config.js') as {
  theme: {
    extend: {
      animation: Record<string, string>;
      keyframes: Record<string, Record<string, Record<string, string>>>;
    };
  };
};
for (const [name, stops] of Object.entries(tailwind.theme.extend.keyframes)) {
  const f: Frames = { from: null, to: null };
  for (const [key, decls] of Object.entries(stops)) {
    const keys = key.split(',').map((k) => k.trim());
    const pairs = Object.entries(decls).map(
      ([k, v]) => [k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`), String(v)] as [string, string],
    );
    if (keys.includes('from') || keys.includes('0%')) f.from = body(pairs);
    if (keys.includes('to') || keys.includes('100%')) f.to = body(pairs);
  }
  if (!frames.has(name)) frames.set(name, f);
}

const namesIn = (value: string) =>
  postcss.list.comma(value).flatMap((part) => postcss.list.space(part).filter((t) => frames.has(t)));

// A one-shot whose first and last frames match (or are both hidden) shows its
// meaning only in between: completing it at once erases it.
function middleOnly(names: string[]): boolean {
  return names.some((n) => {
    const f = frames.get(n);
    if (!f || f.from === null || f.to === null) return false;
    if (f.from === f.to) return true;
    const hidden = (b: string) => b.split(';').includes('opacity:0');
    return hidden(f.from) && hidden(f.to);
  });
}

interface Scanned {
  key: string;
  loop: boolean;
  names: string[];
}
const scanned: Scanned[] = [];
for (const { file, root: sheet } of sheets) {
  sheet.walkRules((rule) => {
    if (rule.parent?.type === 'atrule') {
      const at = rule.parent as AtRule;
      if (/keyframes$/.test(at.name) || /prefers-reduced-motion/.test(at.params)) return;
    }
    let starts = false;
    let loop = false;
    const names: string[] = [];
    for (const d of rule.nodes) {
      if (d.type !== 'decl') continue;
      const v = d.value.trim();
      if (d.prop === 'animation' && v !== 'none') {
        starts = true;
        names.push(...namesIn(v));
      }
      if (d.prop === 'animation-name' && v !== 'none') {
        starts = true;
        names.push(...postcss.list.comma(v));
      }
      if ((d.prop === 'animation' || d.prop === 'animation-iteration-count') && /\binfinite\b/.test(v)) loop = true;
    }
    if (starts || loop) scanned.push({ key: `${file}|${norm(rule.selectors.join(', '))}`, loop, names });
  });
}

function rulesStarting(prefix: string): Rule[] {
  const out: Rule[] = [];
  globals.walkRules((r) => {
    if (r.selectors.some((s) => norm(s).startsWith(prefix))) out.push(r);
  });
  return out;
}

function declOf(rule: Rule, prop: string): { value: string; important: boolean } | null {
  for (const d of rule.nodes) {
    if (d.type === 'decl' && d.prop === prop) return { value: d.value.trim(), important: !!d.important };
  }
  return null;
}

// The selectors inside the `:not(` that `prefix` ends with.
function notList(selector: string, prefix: string): string[] {
  const s = norm(selector);
  const start = s.indexOf(prefix) + prefix.length;
  let depth = 1;
  let i = start;
  for (; i < s.length && depth > 0; i++) {
    if (s[i] === '(') depth++;
    else if (s[i] === ')') depth--;
  }
  return postcss.list.comma(s.slice(start, i - 1)).map(norm);
}

function capOf(prefix: string): { rule: Rule; list: string[] } {
  const rules = rulesStarting(prefix);
  expect(rules, prefix).toHaveLength(1);
  const rule = rules[0];
  const selectors = rule.selectors.map(norm);
  expect(selectors, prefix).toHaveLength(3);
  expect(selectors.some((s) => s.endsWith('::before')), `${prefix} ::before`).toBe(true);
  expect(selectors.some((s) => s.endsWith('::after')), `${prefix} ::after`).toBe(true);
  const lists = new Set(selectors.map((s) => notList(s, prefix).join(', ')));
  expect(lists.size, `${prefix} selectors share one exempt list`).toBe(1);
  return { rule, list: notList(selectors[0], prefix) };
}

const sorted = (xs: Iterable<string>) => [...new Set(xs)].sort();

const codeFiles = walk(
  src,
  (n) => /\.(ts|tsx)$/.test(n) && !/\.test\.tsx?$/.test(n) && !n.endsWith('.d.ts'),
).map((p) => ({ file: rel(p), text: readFileSync(p, 'utf8') }));
const codeLines = (text: string) =>
  text.split(/\r?\n/).filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l));

describe('every CSS animation has a decision for the cheap-draw modes', () => {
  it('lists every animation in a stylesheet, and nothing that is gone', () => {
    const keys = new Set(scanned.map((s) => s.key));
    expect({
      missing: sorted(scanned.map((s) => s.key).filter((k) => !(k in CSS_ROWS))),
      stale: sorted(Object.keys(CSS_ROWS).filter((k) => !keys.has(k))),
    }).toEqual({ missing: [], stale: [] });
  });

  it('gives loops and one-shots a decision of their own kind', () => {
    const wrong = scanned
      .filter((s) => s.key in CSS_ROWS)
      .filter((s) => !(s.loop ? LOOP_DECISIONS : ONCE_DECISIONS).has(CSS_ROWS[s.key].d))
      .map((s) => `${s.key} (${s.loop ? 'loop' : 'one-shot'}: ${CSS_ROWS[s.key].d})`);
    expect(wrong).toEqual([]);
  });

  it('never completes at once a one-shot whose meaning is in its middle frames', () => {
    const erased = scanned
      .filter((s) => CSS_ROWS[s.key]?.d === 'instant-ok' && !CSS_ROWS[s.key].why && middleOnly(s.names))
      .map((s) => s.key);
    for (const [name, row] of Object.entries(TAILWIND_ROWS)) {
      if (row.d === 'instant-ok' && !row.why && middleOnly([name])) erased.push(`tailwind ${name}`);
    }
    expect(erased).toEqual([]);
  });

  it('writes every named rest under the cheap-draw attribute', () => {
    const selectors = new Set<string>();
    for (const { root: sheet } of sheets) sheet.walkRules((r) => r.selectors.forEach((s) => selectors.add(norm(s))));
    const missing = Object.entries(CSS_ROWS)
      .filter(([, row]) => row.d === 'rest')
      .filter(([, row]) => !selectors.has(`html[data-sn-low-power] ${row.cover}`))
      .map(([key, row]) => `${key} -> ${row.cover}`);
    expect(missing).toEqual([]);
  });

  it('exempts exactly the loaders, the chat content and the cues', () => {
    const quiet = capOf(QUIET).list;
    const perf = capOf(PERF).list;
    expect(perf).toEqual(quiet);
    const covers = Object.values(CSS_ROWS)
      .filter((row) => EXEMPT_DECISIONS.has(row.d))
      .map((row) => row.cover!);
    expect(sorted(quiet)).toEqual(sorted([...covers, ...TSX_EXEMPT]));
  });

  it('lists every Tailwind config animation with a decision of its kind', () => {
    const config = tailwind.theme.extend.animation;
    expect(sorted(Object.keys(config))).toEqual(sorted(Object.keys(TAILWIND_ROWS)));
    for (const [name, value] of Object.entries(config)) {
      const loop = /\binfinite\b/.test(value);
      expect((loop ? LOOP_DECISIONS : ONCE_DECISIONS).has(TAILWIND_ROWS[name].d), name).toBe(true);
    }
  });
});

describe('Motion Off and the cheap-draw caps', () => {
  it('Motion Off stops loops and keeps only the loaders moving', () => {
    const { rule, list } = capOf(OFF);
    expect(sorted(list)).toEqual(sorted(OFF_EXEMPT));
    expect(declOf(rule, 'animation-iteration-count')).toEqual({ value: '1', important: true });
    expect(declOf(rule, 'animation-duration')).toEqual({ value: '0.001ms', important: true });
  });

  it('the Penrose march and the offline drift restate their endless loops', () => {
    for (const selector of [
      'html[data-motion="off"] .penrose-march .pm-cube',
      'html[data-motion] div.offline-bounce-x',
      'html[data-motion] div.offline-bounce-y',
    ]) {
      const rules = rulesStarting(selector).filter((r) => r.selectors.map(norm).includes(selector));
      expect(rules, selector).toHaveLength(1);
      expect(declOf(rules[0], 'animation-iteration-count'), selector).toEqual({
        value: 'infinite',
        important: true,
      });
    }
  });

  it('the quiet cap plays every loop once; Performance mode also completes them at once', () => {
    expect(declOf(capOf(QUIET).rule, 'animation-iteration-count')).toEqual({ value: '1', important: true });
    const perf = capOf(PERF).rule;
    expect(declOf(perf, 'animation-duration')).toEqual({ value: '0s', important: true });
    expect(declOf(perf, 'animation-delay')).toEqual({ value: '0s', important: true });
  });

  it('no cheap-draw rule starts or renames an animation', () => {
    const bad: string[] = [];
    for (const { file, root: sheet } of sheets) {
      sheet.walkRules((r) => {
        if (!r.selectors.some((s) => CHEAP_ATTRS.some((a) => s.includes(a)))) return;
        for (const d of r.nodes) {
          if (d.type !== 'decl') continue;
          if ((d.prop === 'animation' || d.prop === 'animation-name') && d.value.trim() !== 'none') {
            bad.push(`${file}|${norm(r.selector)} ${d.prop}: ${d.value}`);
          }
        }
      });
    }
    expect(bad).toEqual([]);
  });

  it('Freeze emotes stops the emote modifiers', () => {
    const selectors = new Set<string>();
    globals.walkRules((r) => {
      if (declOf(r, 'animation')?.value === 'none') r.selectors.forEach((s) => selectors.add(norm(s)));
    });
    expect(selectors.has(`html[data-sn-still-chat] ${FFZ}`)).toBe(true);
    expect(selectors.has(`html[data-sn-still-chat] ${BTTV}`)).toBe(true);
  });
});

describe('loops outside the stylesheets', () => {
  it('a decorative animate-pulse carries sn-decor; an unmarked one is a listed loader', () => {
    const unlisted: string[] = [];
    const stray: string[] = [];
    const loaderFilesUsed = new Set<string>();
    for (const { file, text } of codeFiles) {
      codeLines(text).forEach((line) => {
        const pulse = /\banimate-pulse\b/.test(line);
        const decor = /\bsn-decor\b/.test(line);
        if (decor && !pulse) stray.push(`${file}: ${line.trim()}`);
        if (!pulse || decor) return;
        if (PULSE_LOADER_FILES.has(file)) loaderFilesUsed.add(file);
        else unlisted.push(`${file}: ${line.trim()}`);
      });
    }
    expect({
      unlisted,
      stray,
      stale: sorted([...PULSE_LOADER_FILES].filter((f) => !loaderFilesUsed.has(f))),
    }).toEqual({ unlisted: [], stray: [], stale: [] });
  });

  it('lists every arbitrary animate-[...] class, inline loop and framer loop', () => {
    const found = (re: RegExp) =>
      sorted(codeFiles.filter(({ text }) => codeLines(text).some((l) => re.test(l))).map((f) => f.file));
    expect(found(/animate-\[/)).toEqual(sorted(Object.keys(ARBITRARY_ANIMATE)));
    expect(found(/animation\s*:\s*['"`]?[^;'"`\n]*\binfinite\b/)).toEqual(sorted(Object.keys(INLINE_LOOPS)));
    expect(found(/repeat:\s*Infinity\b/)).toEqual(sorted(Object.keys(FRAMER_LOOPS)));
  });

  it('an inline loader is marked sn-loader', () => {
    for (const [file, d] of Object.entries(INLINE_LOOPS)) {
      if (d !== 'keeps') continue;
      const text = codeFiles.find((f) => f.file === file)!.text;
      expect(text, file).toMatch(/className="sn-loader"/);
    }
  });

  it('every framer loop holds still in the cheap look and under Motion Off', () => {
    for (const [file, how] of Object.entries(FRAMER_LOOPS)) {
      const text = codeFiles.find((f) => f.file === file)!.text;
      if (how === 'words') {
        expect(text, file).toMatch(/motionMode !== 'full'/);
        continue;
      }
      expect(text, file).toMatch(/const holdStill = lowPower \|\| motionOff;/);
      expect(text, file).toMatch(/useLowPower\(\)/);
      const loops = text.match(/repeat:\s*Infinity\b/g) ?? [];
      const gatedAnimate = text.match(/animate=\{\s*holdStill\s*\?\s*\{[^}]*\}\s*:\s*\{[^}]*\}\s*\}/g) ?? [];
      const gatedTransition =
        text.match(
          /transition=\{\s*holdStill\s*\?\s*\{\s*duration:\s*0\s*\}\s*:\s*\{[^}]*repeat:\s*Infinity[^}]*\}\s*\}/g,
        ) ?? [];
      expect(loops.length, `${file} loops`).toBeGreaterThan(0);
      expect(gatedTransition.length, `${file}: every repeat sits in the ungated branch`).toBe(loops.length);
      expect(gatedAnimate.length, `${file}: animate swaps with transition`).toBe(loops.length);
    }
  });
});
