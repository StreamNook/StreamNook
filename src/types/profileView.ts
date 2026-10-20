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

export interface ProfileBadgeGroup {
  /** "Twitch", "7TV", "Chat clients" or "StreamNook". */
  title: string;
  badges: ProfileBadge[];
}

/** One overview tile on a member's sheet. `key` is the stat's visibility key
 *  (`hours`, `messages`, `streams`, `favorite_channel`, `subs_gifted`,
 *  `twitch_age`, `followers`, `views`), which also picks its icon. */
export interface MemberStatView {
  key: string;
  label: string;
  value: string;
  caption?: string;
}

/** One earned accolade: the desktop's lucide icon name and its two gradient
 *  stops (`#rrggbb`). */
export interface AccoladeMedalView {
  id: string;
  label: string;
  icon: string;
  colors: string[];
}

export interface TopEmoteView {
  id: string;
  name: string;
  provider: string;
  url: string;
  count: number;
}

/** A member's relics, overview (ending on their profile views), earned
 *  accolades and top emotes, with what they hide already left out. */
export interface MemberProfileView {
  /** Their relics, the featured one first; the sheet's StreamNook block leads
   *  with them. */
  relics: ProfileBadge[];
  stats: MemberStatView[];
  accolades: AccoladeMedalView[];
  top_emotes: TopEmoteView[];
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
