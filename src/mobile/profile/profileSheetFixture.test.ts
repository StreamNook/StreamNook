// The sheet Rust builds, checked at runtime against what the Android types
// promise: every key is known, every required key is there, every value has the
// right kind. The golden file is written by the Rust test
// `profile_view::tests::golden_member_sheet` (UPDATE_FIXTURES=1), so a field
// added on one side only fails here and not on someone's phone.
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { SAMPLE_SHEET } from './profileSheetSample';

type Errors = string[];
type Check = (v: unknown, path: string) => Errors;

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const prim =
  (t: 'string' | 'number' | 'boolean'): Check =>
  (v, p) =>
    typeof v === t ? [] : [`${p}: expected ${t}`];
const str = prim('string');
const num = prim('number');
const bool = prim('boolean');
/** A shape owned by another registry (paint, Atmosphere, picture): an object, keys not ours. */
const opaque: Check = (v, p) => (isObj(v) ? [] : [`${p}: expected object`]);
const arrayOf =
  (item: Check): Check =>
  (v, p) =>
    Array.isArray(v) ? v.flatMap((x, i) => item(x, `${p}[${i}]`)) : [`${p}: expected array`];
const tupleOf =
  (n: number, item: Check): Check =>
  (v, p) =>
    Array.isArray(v) && v.length === n ? v.flatMap((x, i) => item(x, `${p}[${i}]`)) : [`${p}: expected ${n} items`];
const oneOf =
  (...values: string[]): Check =>
  (v, p) =>
    typeof v === 'string' && values.includes(v) ? [] : [`${p}: expected ${values.join(' | ')}`];
const shape =
  (required: Record<string, Check>, optional: Record<string, Check> = {}): Check =>
  (v, p) => {
    if (!isObj(v)) return [`${p}: expected object`];
    const errors: Errors = [];
    for (const key of Object.keys(v)) {
      const check = required[key] ?? optional[key];
      if (!check) errors.push(`${p}.${key}: unknown key`);
      else errors.push(...check(v[key], `${p}.${key}`));
    }
    for (const key of Object.keys(required)) if (!(key in v)) errors.push(`${p}.${key}: missing`);
    return errors;
  };

const badge = shape(
  { id: str, url: str },
  { fallbacks: arrayOf(str), title: str, name: str, description: str, provider: str },
);
// Every optional field is a missing key, never null (planner X6): `str` and the
// shapes below reject null, so a null here fails.
const stat = shape({ key: str, label: str, value: str }, { caption: str, detail: str });
const section = shape({ key: str, title: str, tiles: arrayOf(stat) });
const medal = shape({
  id: str,
  label: str,
  icon: str,
  colors: tupleOf(2, str),
  earned: bool,
  secret: bool,
  hint: str,
});
const memberProfile = shape(
  {
    relics: arrayOf(badge),
    top_emotes: arrayOf(
      shape({ id: str, name: str, provider: str, url: str, count: num, count_text: str }),
    ),
  },
  {
    hours: shape({ label: str, value: str, unit: str }, { roast: str }),
    twitch: section,
    lifetime: section,
    views: str,
    emote_totals: str,
    accolades: shape({ earned: num, total: num, items: arrayOf(medal) }),
  },
);

/** `ProfileSheetView`, exactly. `paint`, `cologne`, `picture` and `atmosphere` are
 *  other registries' shapes (opaque here); `paint` and `cologne` are Rust-only. */
export const validateSheet: Check = shape(
  {
    user_id: str,
    login: str,
    display_name: str,
    name_color: str,
    groups: arrayOf(shape({ title: str, badges: arrayOf(badge) })),
  },
  {
    shown_login: str,
    avatar_url: str,
    picture: opaque,
    paint: opaque,
    paint_def: opaque,
    cologne: opaque,
    atmosphere: opaque,
    streamnook: shape({ image_url: str, label: str }, { slug: str }),
    member: shape({ number: num, tier: str, label: str, accent: str, aura: num }),
    member_profile: memberProfile,
    page_link: shape({ href: str, url: str, prefix: str, name: str, accent: str }),
    worn: arrayOf(badge),
    seventv_user_id: str,
    theme: oneOf('tier', 'paint', 'atmosphere'),
    frame: shape({ url: str, slice: tupleOf(4, num), border: tupleOf(2, num) }),
    accent: str,
    caption: str,
  },
);

const check = (v: unknown) => validateSheet(v, 'sheet');
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

describe('the sheet validator', () => {
  it('accepts a sheet with every field set', () => {
    expect(check(SAMPLE_SHEET)).toEqual([]);
  });

  it('accepts a missing optional key', () => {
    const s = clone(SAMPLE_SHEET) as unknown as Record<string, Record<string, unknown>>;
    delete s.member_profile.hours;
    delete s.member_profile.views;
    delete s.member_profile.accolades;
    expect(check(s)).toEqual([]);
  });

  it('rejects null for an optional key: Rust leaves the key out', () => {
    const s = clone(SAMPLE_SHEET) as unknown as Record<string, Record<string, unknown>>;
    s.member_profile.hours = null;
    s.member_profile.views = null;
    expect(check(s)).toEqual(
      expect.arrayContaining(['sheet.member_profile.hours: expected object', 'sheet.member_profile.views: expected string']),
    );
    const t = clone(SAMPLE_SHEET) as unknown as { member_profile: { twitch: { tiles: Record<string, unknown>[] } } };
    t.member_profile.twitch.tiles[0].detail = null;
    expect(check(t)).toContain('sheet.member_profile.twitch.tiles[0].detail: expected string');
  });

  it('requires the hours heading', () => {
    const s = clone(SAMPLE_SHEET) as unknown as { member_profile: { hours: Record<string, unknown> } };
    delete s.member_profile.hours.label;
    expect(check(s)).toContain('sheet.member_profile.hours.label: missing');
  });

  it('rejects a key the types do not name', () => {
    const s = clone(SAMPLE_SHEET) as unknown as Record<string, unknown>;
    s.stats = [];
    expect(check(s)).toContain('sheet.stats: unknown key');
  });

  it('rejects a new key hidden in a nested object', () => {
    const s = clone(SAMPLE_SHEET) as unknown as { member_profile: { top_emotes: Record<string, unknown>[] } };
    s.member_profile.top_emotes[0].extra = 1;
    expect(check(s)).toContain('sheet.member_profile.top_emotes[0].extra: unknown key');
  });

  it('rejects a missing required key and a wrong kind', () => {
    const s = clone(SAMPLE_SHEET) as unknown as { member: Record<string, unknown> };
    delete s.member.aura;
    expect(check(s)).toContain('sheet.member.aura: missing');
    const t = clone(SAMPLE_SHEET) as unknown as { theme: unknown };
    t.theme = 'sparkle';
    expect(check(t)).toContain('sheet.theme: expected tier | paint | atmosphere');
  });
});

const NEW_SHEET_KEYS = ['page_link', 'worn', 'seventv_user_id', 'theme', 'frame', 'accent', 'caption'];
const NEW_PROFILE_KEYS = ['hours', 'twitch', 'lifetime', 'views', 'emote_totals', 'accolades'];

// Commit B (7.2) lands the file before this card, so a missing file is a failure.
describe('the golden member sheet from Rust', () => {
  const path = resolve(__dirname, 'fixtures', 'profileSheet.member.json');
  const load = () => JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>;

  it('exists', () => {
    expect(existsSync(path), 'run UPDATE_FIXTURES=1 cargo test golden_member_sheet in src-tauri').toBe(true);
  });

  it('is exactly ProfileSheetView', () => {
    expect(check(load())).toEqual([]);
  });

  it('sets every field this plan added', () => {
    const sheet = load();
    for (const key of NEW_SHEET_KEYS) expect(sheet, `sheet.${key}`).toHaveProperty(key);
    expect(sheet.member).toHaveProperty('aura');
    const profile = sheet.member_profile as Record<string, unknown>;
    for (const key of NEW_PROFILE_KEYS) expect(profile, `member_profile.${key}`).toHaveProperty(key);
    expect(profile).not.toHaveProperty('stats');
    expect(profile).not.toHaveProperty('featured_relic');
  });
});
