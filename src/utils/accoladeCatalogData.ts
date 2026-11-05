// The accolade catalog (accolades.catalog.json), parsed once and typed: every
// accolade the wall can show with its colors, rule, hints and window, the stat
// tiles' labels and icons, the meme member numbers with their names, and the
// section titles. The phones read the same file through Rust, and the member
// page on streamnook.app runs the same shared cases against its own copy of the
// rules.
//
// The parse is strict: an unknown key, a bad color or a duplicate id throws
// when the module loads, so a bad edit to the file fails at once instead of
// drawing a blank medallion.
import raw from './accolades.catalog.json';

export type AccoladeKind = 'base' | 'secret' | 'offline' | 'seasonal' | 'cake' | 'event';
export type RuleStat = 'messages' | 'hours' | 'streams' | 'drops' | 'points' | 'cosmetics' | 'years';
export type RuleSpecial = 'first_hundred' | 'completionist' | 'triple' | 'palindrome' | 'nice' | 'saved';
export type AccoladeRule = { stat: RuleStat; n: number } | { special: RuleSpecial };

export interface SeasonWindow {
  from_m: number;
  from_d: number;
  to_m: number;
  to_d: number;
}

export interface AccoladeDef {
  id: string;
  label: string;
  icon: string;
  colors: [string, string];
  kind: AccoladeKind;
  rule: AccoladeRule;
  progress?: string;
  goal: string;
  hint?: string;
  hint_earned?: string;
  window?: SeasonWindow;
  ended?: boolean;
}

export interface StatDef {
  key: string;
  label: string;
  icon: string;
  caption?: string;
}

export interface AccoladeCatalog {
  v: 1;
  titles: Record<string, string>;
  stats: StatDef[];
  member_number_names: { exact: Record<string, string>; ends_in_67: string };
  accolades: AccoladeDef[];
}

const KINDS: readonly string[] = ['base', 'secret', 'offline', 'seasonal', 'cake', 'event'];
const STATS: readonly string[] = ['messages', 'hours', 'streams', 'drops', 'points', 'cosmetics', 'years'];
const SPECIALS: readonly string[] = ['first_hundred', 'completionist', 'triple', 'palindrome', 'nice', 'saved'];
const DEF_KEYS = new Set(['id', 'label', 'icon', 'colors', 'kind', 'rule', 'progress', 'goal', 'hint', 'hint_earned', 'window', 'ended']);
const STAT_KEYS = new Set(['key', 'label', 'icon', 'caption']);
const WINDOW_KEYS = ['from_m', 'from_d', 'to_m', 'to_d'];
const HEX = /^#[0-9a-f]{6}$/;

const fail = (where: string, why: string): never => {
  throw new Error(`accolades.catalog.json: ${where}: ${why}`);
};
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown, where: string): string => (typeof v === 'string' && v !== '' ? v : fail(where, 'expected a non-empty string'));
const onlyKeys = (o: Record<string, unknown>, allowed: ReadonlySet<string>, where: string): void => {
  for (const k of Object.keys(o)) if (!allowed.has(k)) fail(where, `unknown key "${k}"`);
};

function parseRule(v: unknown, where: string): AccoladeRule {
  if (!isObject(v)) return fail(where, 'rule must be an object');
  if ('stat' in v) {
    onlyKeys(v, new Set(['stat', 'n']), where);
    const stat = text(v.stat, `${where}.stat`);
    if (!STATS.includes(stat)) fail(where, `unknown stat "${stat}"`);
    if (typeof v.n !== 'number' || !Number.isInteger(v.n) || v.n <= 0) fail(where, 'n must be a positive integer');
    return { stat: stat as RuleStat, n: v.n as number };
  }
  onlyKeys(v, new Set(['special']), where);
  const special = text(v.special, `${where}.special`);
  if (!SPECIALS.includes(special)) fail(where, `unknown special "${special}"`);
  return { special: special as RuleSpecial };
}

function parseDef(v: unknown, at: number): AccoladeDef {
  const where = `accolades[${at}]`;
  if (!isObject(v)) return fail(where, 'expected an object');
  onlyKeys(v, DEF_KEYS, where);
  const id = text(v.id, `${where}.id`);
  const kind = text(v.kind, `${where}.kind`);
  if (!KINDS.includes(kind)) fail(where, `unknown kind "${kind}"`);
  const colors = v.colors;
  if (!Array.isArray(colors) || colors.length !== 2 || !colors.every((c) => typeof c === 'string' && HEX.test(c))) {
    fail(where, 'colors must be two #rrggbb strings');
  }
  const rule = parseRule(v.rule, `${where}.rule`);
  const def: AccoladeDef = {
    id,
    label: text(v.label, `${where}.label`),
    icon: text(v.icon, `${where}.icon`),
    colors: colors as [string, string],
    kind: kind as AccoladeKind,
    rule,
    goal: text(v.goal, `${where}.goal`),
  };
  if (kind === 'base') {
    if (('stat' in rule) !== (v.progress !== undefined)) fail(where, 'a base accolade has a progress label exactly when its rule reads a stat');
  } else if (v.progress !== undefined || v.hint === undefined) {
    fail(where, 'only a base accolade has progress, and every other kind has a hint');
  }
  if (v.progress !== undefined) def.progress = text(v.progress, `${where}.progress`);
  if (v.hint !== undefined) def.hint = text(v.hint, `${where}.hint`);
  if ((kind === 'event') !== (v.hint_earned !== undefined)) fail(where, 'hint_earned belongs to events only, and every event has one');
  if (v.hint_earned !== undefined) def.hint_earned = text(v.hint_earned, `${where}.hint_earned`);
  if ((kind === 'seasonal') !== (v.window !== undefined)) fail(where, 'window belongs to seasonal accolades only, and every one has one');
  if (v.window !== undefined) {
    const w = v.window;
    if (!isObject(w)) return fail(where, 'window must be an object');
    onlyKeys(w, new Set(WINDOW_KEYS), `${where}.window`);
    for (const k of WINDOW_KEYS) if (typeof w[k] !== 'number' || !Number.isInteger(w[k])) fail(where, `window.${k} must be an integer`);
    def.window = w as unknown as SeasonWindow;
  }
  if (v.ended !== undefined) {
    if (v.ended !== true) fail(where, 'ended is true or absent');
    def.ended = true;
  }
  return def;
}

/** Checks the shape of a catalog and returns it typed; throws on any deviation. */
export function parseCatalog(input: unknown): AccoladeCatalog {
  if (!isObject(input)) return fail('file', 'expected an object');
  onlyKeys(input, new Set(['v', 'titles', 'stats', 'member_number_names', 'accolades']), 'file');
  if (input.v !== 1) fail('file', 'v must be 1');
  if (!isObject(input.titles)) fail('titles', 'expected an object');
  const titles: Record<string, string> = {};
  for (const [k, t] of Object.entries(input.titles as Record<string, unknown>)) titles[k] = text(t, `titles.${k}`);
  for (const k of ['hidden_secret_hint', 'collected']) if (!titles[k]) fail('titles', `missing "${k}"`);

  if (!Array.isArray(input.stats)) fail('stats', 'expected an array');
  const stats = (input.stats as unknown[]).map((s, i): StatDef => {
    if (!isObject(s)) return fail(`stats[${i}]`, 'expected an object');
    onlyKeys(s, STAT_KEYS, `stats[${i}]`);
    const def: StatDef = { key: text(s.key, `stats[${i}].key`), label: text(s.label, `stats[${i}].label`), icon: text(s.icon, `stats[${i}].icon`) };
    if (s.caption !== undefined) def.caption = text(s.caption, `stats[${i}].caption`);
    return def;
  });

  const names = input.member_number_names;
  if (!isObject(names) || !isObject(names.exact)) return fail('member_number_names', 'expected { exact, ends_in_67 }');
  onlyKeys(names, new Set(['exact', 'ends_in_67']), 'member_number_names');
  const exact: Record<string, string> = {};
  for (const [k, name] of Object.entries(names.exact)) {
    if (!/^[1-9]\d*$/.test(k)) fail('member_number_names.exact', `"${k}" is not a member number`);
    exact[k] = text(name, `member_number_names.exact.${k}`);
  }

  if (!Array.isArray(input.accolades)) fail('accolades', 'expected an array');
  const accolades = (input.accolades as unknown[]).map(parseDef);
  const seen = new Set<string>();
  for (const a of accolades) {
    if (seen.has(a.id)) fail('accolades', `duplicate id "${a.id}"`);
    seen.add(a.id);
  }
  return { v: 1, titles, stats, member_number_names: { exact, ends_in_67: text(names.ends_in_67, 'member_number_names.ends_in_67') }, accolades };
}

export const CATALOG: AccoladeCatalog = parseCatalog(raw);
