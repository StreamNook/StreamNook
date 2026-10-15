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
  /** "Twitch", "7TV" or "Chat clients". */
  title: string;
  badges: ProfileBadge[];
}

/** A member's StreamNook badge. `image_url` may be `streamnook-house:<slug>`,
 *  art the apps ship (the page draws it through StreamNookBadge instead). */
export interface SnMemberBadge {
  slug?: string;
  image_url: string;
  label: string;
}

export interface ProfileSheetView {
  user_id: string;
  login: string;
  display_name: string;
  shown_login?: string;
  avatar_url?: string;
  name_color: string;
  /** The worn 7TV paint as 7TV defines it. */
  paint_def?: PaintV4;
  groups: ProfileBadgeGroup[];
  streamnook?: SnMemberBadge;
}

export interface OwnIdentityView {
  user_id: string;
  login: string;
  display_name: string;
  shown_login?: string;
  paint_def?: PaintV4;
  /** Worn, in order, images only, at most eight. */
  badges: ProfileBadge[];
  streamnook?: SnMemberBadge;
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
}
