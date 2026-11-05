// The profile sheet's small rules, pure so they are tested without a screen. The
// iPhone keeps the same seams in SNKit/ProfileParts.swift and runs the same cases.
import type { PlateSlot } from '../../components/profile/profileLink';
import type {
  AccoladeMedalView,
  AtmosphereView,
  FrameView,
  MemberRank,
  MemberStatView,
  PageLink,
  ProfileBadge,
  ProfileSheetView,
  TopEmoteView,
} from '../../types/profileView';

/** How the sheet's StreamNook page plate draws. A link shows. Without one, a
 *  sheet whose fresh answer has landed shows nothing (and a reserved slot
 *  collapses). Until then the slot is held while the member list has not loaded
 *  or the person is a member, so the plate landing moves nothing. */
export function plateSlot(
  registryLoaded: boolean,
  isMember: boolean,
  link: PageLink | null,
  freshAnswered: boolean,
): PlateSlot {
  if (link) return 'shown';
  if (freshAnswered) return 'hidden';
  return !registryLoaded || isMember ? 'reserved' : 'hidden';
}

export interface AuraSpec {
  kind: 'atmosphere' | 'aura' | 'none';
  /** The profile accent for an Atmosphere, the rank color for an aura. */
  color?: string;
  /** An aura's strength, 0 to 1. */
  alpha?: number;
}

/** What fills the whole sheet behind the content: their Atmosphere when one is
 *  worn, else their rank's aura, else nothing. (`theme` rides along with the
 *  contract; the worn Atmosphere is what decides.) */
export function auraFor(
  _theme: ProfileSheetView['theme'],
  atmosphere: AtmosphereView | null | undefined,
  accent: string | undefined,
  rank: MemberRank | null | undefined,
): AuraSpec {
  if (atmosphere) return accent ? { kind: 'atmosphere', color: accent } : { kind: 'atmosphere' };
  if (rank) return { kind: 'aura', color: rank.accent, alpha: rank.aura };
  return { kind: 'none' };
}

export interface Insets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/** The Frame's border widths: vertical for top and bottom, horizontal for left
 *  and right, times the screen scale. */
export function frameInsets(frame: Pick<FrameView, 'border'>, scale: number): Insets {
  const [vertical, horizontal] = frame.border;
  return { top: vertical * scale, right: horizontal * scale, bottom: vertical * scale, left: horizontal * scale };
}

export type PanelKind = 'accolade' | 'emote' | 'badge' | 'paint' | 'atmosphere';

/** What the in-place panel a tap opens says. */
export interface Panel {
  title: string;
  detail: string;
}

const SOFT_HYPHEN = /­/g;

export function panelFor(kind: 'accolade', item: Pick<AccoladeMedalView, 'label' | 'hint'>): Panel;
export function panelFor(kind: 'emote', item: Pick<TopEmoteView, 'name' | 'count_text' | 'provider'>): Panel;
export function panelFor(kind: 'badge', item: Pick<ProfileBadge, 'title' | 'name' | 'description'>): Panel;
export function panelFor(kind: 'paint' | 'atmosphere', item: { name: string }): Panel;
export function panelFor(kind: PanelKind, item: unknown): Panel {
  switch (kind) {
    case 'accolade': {
      const a = item as Pick<AccoladeMedalView, 'label' | 'hint'>;
      return { title: a.label.replace(SOFT_HYPHEN, ''), detail: a.hint };
    }
    case 'emote': {
      const e = item as Pick<TopEmoteView, 'name' | 'count_text' | 'provider'>;
      return { title: e.name, detail: `${e.count_text} · ${e.provider.toUpperCase()}` };
    }
    case 'badge': {
      const b = item as Pick<ProfileBadge, 'title' | 'name' | 'description'>;
      return { title: b.title || b.name || 'Badge', detail: b.description ?? '' };
    }
    case 'paint':
      return { title: (item as { name: string }).name, detail: '7TV paint' };
    case 'atmosphere':
      return { title: (item as { name: string }).name, detail: 'Atmosphere' };
  }
}

/** How many emote tiles the sheet draws. */
export const EMOTE_TILE_LIMIT = 12;

/** The emote tiles. Rust sends up to 16 emotes with art; art that failed every
 *  URL goes first, then the first twelve, so a broken image is replaced by the
 *  next emote and not left a hole. */
export function emoteTiles(emotes: readonly TopEmoteView[], failed: ReadonlySet<string>): TopEmoteView[] {
  return emotes.filter((e) => !failed.has(e.id)).slice(0, EMOTE_TILE_LIMIT);
}

/** `#rrggbb` as three channels, or null for anything else. */
export function hexRgb(hex: string | null | undefined): [number, number, number] | null {
  const m = /^#?([0-9a-fA-F]{6})$/.exec((hex ?? '').trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** The profile accent at `alpha` as a color, or undefined without a valid accent. */
export function accentTint(accent: string | null | undefined, alpha: number): string | undefined {
  const rgb = hexRgb(accent);
  return rgb ? `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${alpha})` : undefined;
}

/** An accent-tinted border as an inline style, or undefined to keep the theme's. */
export function accentBorder(
  accent: string | null | undefined,
  alpha: number,
): { borderColor: string } | undefined {
  const color = accentTint(accent, alpha);
  return color ? { borderColor: color } : undefined;
}

/** A tile value's type size: longer values step down so a number in a tile that
 *  cannot grow never clips (the widest tile is about 122 px of text). */
export function statValueSize(value: string): number {
  const n = [...value].length;
  if (n <= 8) return 17;
  if (n <= 11) return 15;
  return 13;
}

/** The headings the phones print that Rust does not send (the hours heading is
 *  `HoursHero.label`; the Twitch and Lifetime headings are `StatSection.title`).
 *  The same words as `utils/accolades.catalog.json` `titles`, which a test
 *  pins; the phones themselves read no catalog. */
export const SECTION_TITLES = { relics: 'Relics', top_emotes: 'Top Emotes', accolades: 'Accolades' } as const;

/** What tapping a stat tile says in place: its label and its detail ("Joined
 *  March 20, 2014" on Twitch age). A tile without a detail has nothing to say. */
export function statPanel(stat: Pick<MemberStatView, 'label' | 'detail'>): Panel | null {
  return stat.detail ? { title: stat.label, detail: stat.detail } : null;
}

/** Which mark the Account type tile draws, from its value. */
export function accountGlyph(value: string): 'verified' | 'twitch' | 'award' {
  if (value === 'Partner' || value === 'Twitch Staff') return 'verified';
  if (value === 'Affiliate') return 'twitch';
  return 'award';
}

/** The device's offset from UTC in minutes, east positive (what Rust's
 *  `tz_offset_min` takes). `Date#getTimezoneOffset` is west positive. */
export function tzOffsetMin(now: Pick<Date, 'getTimezoneOffset'> = new Date()): number {
  return 0 - now.getTimezoneOffset();
}

/** One id per presentation of a sheet: Rust keeps a member's roast line steady
 *  for one id and rolls a new one for the next. */
export function newOpenId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === 'function') return c.randomUUID();
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}
