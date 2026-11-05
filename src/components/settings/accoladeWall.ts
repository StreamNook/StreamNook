// The public accolade wall for one member: which accolades they can still reach,
// which they hold, and the words each medallion carries for someone looking at
// their profile (a number the member hides, or one the public view never reads,
// never appears in a hint, and a locked secret is not listed).
//
// Pure, with no React. Your own wall is not built here: Rust builds it
// (`own_accolades`), counting everything every platform counts. The accolades
// come from accolades.catalog.json; the public cases in accolades.cases.json pin
// the output, and Rust and the streamnook.app member page run the same cases.
import { CATALOG, type AccoladeDef, type RuleStat } from '../../utils/accoladeCatalogData';
import { isPalindrome, outOfReachAccolades } from '../../utils/accoladeReach';
import { memberNumberName } from '../../utils/memberNumberNames';
import { isStatHidden, type ProfileStatKey } from '../../services/profileVisibility';

export interface WallInput {
  messages: number;
  hours: number;
  streams: number;
  /** Drops claimed and channel points held are never read for a public wall; 0. */
  drops: number;
  points: number;
  /** Paints, 7TV badges and StreamNook cosmetics together. */
  cosmetics: number;
  /** Whole years on Twitch. */
  years: number;
  member_number: number | null;
  /** Accolade ids stored for the member (user_accolades). */
  saved: readonly string[];
  /** The member's hidden_sections as stored. */
  hidden: readonly string[];
}

export interface WallItem {
  id: string;
  label: string;
  icon: string;
  colors: [string, string];
  earned: boolean;
  /** A secret accolade reads "???" until it is earned. */
  secret: boolean;
  hint: string;
}

export interface AccoladeWall {
  earned: number;
  total: number;
  /** Earned first, then locked, each group in catalog order. */
  items: WallItem[];
}

const grouped = new Intl.NumberFormat('en-US');
const fmt = (v: number): string => grouped.format(Math.round(v));

/** The stat switch that hides the number behind each stat-based accolade. */
const HIDING_KEY: Partial<Record<RuleStat, ProfileStatKey>> = {
  messages: 'messages',
  hours: 'hours',
  streams: 'streams',
  cosmetics: 'cosmetics',
  years: 'twitch_age',
};

/** Triple Threat: talk, watch and collect until all three run deep. */
const TRIPLE = { messages: 1000, hours: 100, drops: 25 };

export function buildAccoladeWall(input: WallInput): AccoladeWall {
  // The stored set: a season or Cake Day lights only when the server saved it.
  const saved = new Set(input.saved);

  const closed = outOfReachAccolades(input.member_number);
  const meme = memberNumberName(input.member_number);

  /** Whether the numbers alone light an accolade (Completionist is decided below). */
  const reached = (d: AccoladeDef): boolean => {
    const r = d.rule;
    if ('stat' in r) return input[r.stat] >= r.n;
    switch (r.special) {
      case 'first_hundred':
        return input.member_number !== null && input.member_number <= 100;
      case 'triple':
        return input.messages >= TRIPLE.messages && input.hours >= TRIPLE.hours && input.drops >= TRIPLE.drops;
      case 'palindrome':
        return isPalindrome(input.member_number);
      case 'nice':
        return meme !== null;
      default:
        return false;
    }
  };

  // Completionist counts only the base accolades this member can still earn, so
  // OG's first-100 cut never puts it out of reach.
  const baseDone = CATALOG.accolades
    .filter((d) => d.kind === 'base')
    .every((d) => reached(d) || saved.has(d.id) || closed.has(d.id));

  const hintFor = (d: AccoladeDef, earned: boolean): string => {
    const plain = d.hint ?? d.goal;
    switch (d.kind) {
      case 'secret':
        // A public wall never shows the how-to, so it cannot be read off an earned one.
        return CATALOG.titles.hidden_secret_hint;
      case 'seasonal':
      case 'cake':
        return earned ? CATALOG.titles.collected : plain;
      case 'event':
        return earned ? (d.hint_earned ?? plain) : plain;
      case 'offline':
        return plain;
      case 'base': {
        const r = d.rule;
        if (!('stat' in r) || !d.progress) return d.goal;
        const key = HIDING_KEY[r.stat];
        // Never a number the member hid or one the public view does not read
        // (drops, points): the goal alone.
        if (key === undefined || isStatHidden(input.hidden, key)) return d.goal;
        return `${d.progress}: ${fmt(input[r.stat])} / ${fmt(r.n)}`;
      }
    }
  };

  const items: WallItem[] = [];
  for (const d of CATALOG.accolades) {
    const completionist = 'special' in d.rule && d.rule.special === 'completionist';
    const earned = saved.has(d.id) || (completionist ? baseDone : reached(d));
    // An accolade nobody can earn any more is only listed for those who hold it.
    if (!earned && closed.has(d.id)) continue;
    const secret = d.kind === 'secret';
    // A public wall shows no placeholder for a secret that has not been earned.
    if (secret && !earned) continue;
    const nice = 'special' in d.rule && d.rule.special === 'nice';
    items.push({
      id: d.id,
      // One accolade for every meme number; the label is that number's own name.
      label: nice ? (meme ?? d.label) : d.label,
      icon: d.icon,
      colors: [d.colors[0], d.colors[1]],
      earned,
      secret,
      hint: hintFor(d, earned),
    });
  }

  const earnedItems = items.filter((i) => i.earned);
  return {
    earned: earnedItems.length,
    total: items.length,
    items: [...earnedItems, ...items.filter((i) => !i.earned)],
  };
}
