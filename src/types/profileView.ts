// The profile surfaces as Rust builds them (src-tauri/src/services/profile_view.rs):
// the user profile sheet, the viewer's own identity header and the Cosmetics
// screen's two halves. Every rule (badge merging, worn versus owned, the
// atmosphere gates) is Rust's; these only describe its answers.
import type { PaintV4 } from '../services/paintStyle';

export interface ProfileBadge {
  id: string;
  url: string;
  fallbacks?: string[];
  title?: string;
  name?: string;
  description?: string;
  provider?: string;
}

/** A member's StreamNook page as the plate draws it (Rust `member_link::page_link`).
 *  `href` is what opening goes to, `url` is what the plate shows and copies. */
export interface PageLink {
  href: string;
  url: string;
  /** "streamnook.app/" or "streamnook.app/u/". */
  prefix: string;
  name: string;
  /** The rank accent lifted to read on the plate's floor, `#rrggbb`. */
  accent: string;
}

/** The desktop's answer to "may this member card show a plate". */
export interface PageLinkAnswer {
  state: 'member' | 'not_member' | 'not_ready';
  link?: PageLink;
}

/** An equipped Frame: the art and the nine-slice numbers Rust serves
 *  (top, right, bottom, left source slices; vertical and horizontal border). */
export interface FrameView {
  url: string;
  slice: [number, number, number, number];
  border: [number, number];
}

export interface ProfileBadgeGroup {
  /** "Twitch", "7TV", "Chat clients" or "StreamNook". */
  title: string;
  badges: ProfileBadge[];
}

/** One tile of a member's "Twitch" or "Lifetime" section. `key` is the stat's
 *  key in `utils/accolades.catalog.json`, which also picks its icon. `detail` is
 *  set only on `twitch_age` ("Joined March 20, 2014") and shows in the tap
 *  panel. A missing caption or detail is a missing key, never null. */
export interface MemberStatView {
  key: string;
  label: string;
  value: string;
  caption?: string;
  detail?: string;
}

/** The hours hero: its heading, the number, its unit and a roast line when the
 *  member keeps the roast visible. */
export interface HoursHeroView {
  /** "Hours watched". */
  label: string;
  value: string;
  unit: string;
  roast?: string;
}

/** "<Name>'s Twitch" or "Lifetime in StreamNook": a titled grid of tiles. */
export interface StatSectionView {
  /** `twitch` or `lifetime`. */
  key: string;
  title: string;
  tiles: MemberStatView[];
}

/** One accolade on the wall: the desktop's lucide icon name, its two gradient
 *  stops (`#rrggbb`), whether it is lit, and the hint a tap shows. */
export interface AccoladeMedalView {
  id: string;
  label: string;
  icon: string;
  colors: [string, string];
  earned: boolean;
  secret: boolean;
  hint: string;
}

/** The wall: earned ones first, then locked, as the desktop's wall orders them. */
export interface AccoladeWallView {
  earned: number;
  total: number;
  items: AccoladeMedalView[];
}

export interface TopEmoteView {
  id: string;
  name: string;
  provider: string;
  url: string;
  count: number;
  /** The count as the tile prints it. */
  count_text: string;
}

/** A member's StreamNook half, with what they hide already left out. */
export interface MemberProfileView {
  /** Their relics; the first one leads (the equipped one when there is one). */
  relics: ProfileBadge[];
  hours?: HoursHeroView;
  twitch?: StatSectionView;
  lifetime?: StatSectionView;
  /** Profile views as printed ("1,234"), when the member shows them. */
  views?: string;
  /** Up to 16 with art; the sheet draws the first 12 whose art loads. */
  top_emotes: TopEmoteView[];
  /** "12 emotes · 345 used". */
  emote_totals?: string;
  accolades?: AccoladeWallView;
}

/** A member's StreamNook badge. `image_url` may be `streamnook-house:<slug>`,
 *  art the apps ship (the page draws it through StreamNookBadge instead). */
export interface SnMemberBadge {
  slug?: string;
  image_url: string;
  label: string;
}

/** A 7TV profile picture as one surface draws it: the still, and the moving
 *  rendition when the picture moves and the viewer lets this surface animate. */
export interface AvatarPick {
  still: string;
  animated?: string;
}

/** An Atmosphere as the registry serves it (camelCase, the same CSS the
 *  pages paint). Only the fields the profile surfaces read are named. */
export interface AtmosphereView {
  id: string;
  name: string;
  /** "r, g, b". */
  accent: string;
  /** The picker's full-strength CSS background; the identity band draws it. */
  swatch: string;
  baseColor: string;
  chatFrost?: boolean;
}

/** A member's number and rank, drawn on the right of their sheet. */
export interface MemberRank {
  number: number;
  /** `ethereal`, `mythic`, `ascendant`, `founder` or `member`. */
  tier: string;
  /** The rank's name, or a famous number's own ("Nice" for 69). */
  label: string;
  /** How strongly the rank's aura washes the top of the sheet when no
   *  Atmosphere is worn, 0 to 1 (the desktop card's tier aura alpha). */
  aura: number;
  /** The rank's color, `#rrggbb`. */
  accent: string;
}

export interface ProfileSheetView {
  user_id: string;
  login: string;
  display_name: string;
  shown_login?: string;
  avatar_url?: string;
  /** Their 7TV picture, when they set one and the viewer shows 7TV pictures. */
  picture?: AvatarPick;
  name_color: string;
  /** The worn 7TV paint as 7TV defines it. */
  paint_def?: PaintV4;
  groups: ProfileBadgeGroup[];
  streamnook?: SnMemberBadge;
  /** A member's number and rank. */
  member?: MemberRank;
  /** The Atmosphere they wear on their profile, after the wear gate. */
  atmosphere?: AtmosphereView;
  /** A member's relics, overview, accolades and top emotes; their StreamNook
   *  badges ride `groups`. */
  member_profile?: MemberProfileView;
  /** A member's StreamNook page; absent for anyone who is not a member. */
  page_link?: PageLink;
  /** What they wear, Twitch first and StreamNook last, capped (members only). */
  worn?: ProfileBadge[];
  /** Their 7TV account, when they have one linked. */
  seventv_user_id?: string;
  /** Which look the sheet wears: the rank aura, a paint wash or an Atmosphere. */
  theme?: 'tier' | 'paint' | 'atmosphere';
  /** An equipped Frame, drawn around the hero. */
  frame?: FrameView;
  /** The profile accent as `#rrggbb`: section borders and the hero rim. */
  accent?: string;
  /** The worn StreamNook badge's caption under the rank. */
  caption?: string;
}

export interface OwnIdentityView {
  user_id: string;
  login: string;
  display_name: string;
  shown_login?: string;
  paint_def?: PaintV4;
  /** Your Twitch name color, readable as chat shows it; absent when Twitch did not say. */
  name_color?: string;
  /** Your 7TV picture, when you set one and you show 7TV pictures. */
  picture?: AvatarPick;
  /** Your StreamNook page; absent when you are not a member. */
  page_link?: PageLink;
  /** Your Twitch profile banner; absent when the account has none. */
  banner_url?: string;
  /** Worn, in surface order (your StreamNook badge last), images only, capped. */
  badges: ProfileBadge[];
  streamnook?: SnMemberBadge;
  atmosphere?: AtmosphereView;
}

export interface CssFill {
  layers: unknown[];
}

export interface StreamNookPickerView {
  badges: { slug: string; name: string; image_url?: string; worn: boolean }[];
  /** The Default tile (`tier`) first. */
  atmospheres: { id: string; name: string; swatch: CssFill; allowed: boolean; worn: boolean }[];
  theme: string;
  show_chat_toggle: boolean;
  chat_atmosphere: boolean;
}

export interface ChatIdentityBadge {
  id: string;
  version: string;
  title: string;
  image_url: string;
  is_selected: boolean;
}

export interface ExternalPickerView {
  seventv_signed_in: boolean;
  seventv_user_id?: string;
  paints: { id: string; name: string; def: PaintV4; worn: boolean }[];
  seventv_badges: { badge: ProfileBadge; worn: boolean }[];
  drops_signed_in: boolean;
  twitch_selectable: ChatIdentityBadge[];
  twitch_earned: ProfileBadge[];
  /** The chat-client badges you wear (FFZ, BetterTTV, Chatterino and the rest), for the preview row. */
  chat_clients?: ProfileBadge[];
  /** Your Twitch name color (`#rrggbb`), when Twitch says. */
  chat_color?: string;
  /** Twitch's named colors, which every account may pick, in Twitch's order. */
  named_colors: NamedChatColor[];
}

/** One of Twitch's named chat colors: its label, the value Helix takes, its hex. */
export interface NamedChatColor {
  label: string;
  value: string;
  hex: string;
}
