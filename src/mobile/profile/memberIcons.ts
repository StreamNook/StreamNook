// The marks a member's sheet draws for what Rust names (services/member_profile.rs):
// each overview tile's icon and color, keyed by its stat key, and each accolade
// medallion's icon, keyed by the desktop's lucide name. Presentation only; the
// iPhone keeps the same two maps in Cosmetics/MemberSections.swift.
import {
  Cake,
  Clock,
  Eye,
  HandHeart,
  Heart,
  MessageSquare,
  Tv,
  Users,
  type LucideIcon,
} from 'lucide-react';

import { Award, Palette, Sparkles } from 'lucide-react';

/** Each stat tile's icon and the desktop card's color for it; one entry for
 *  every key in `utils/accolades.catalog.json` `stats` (memberIcons.test.ts). */
export const STAT_ICONS: Record<string, { icon: LucideIcon; color: string }> = {
  hours: { icon: Clock, color: '#e6c178' },
  messages: { icon: MessageSquare, color: '#8fd4b4' },
  streams: { icon: Tv, color: '#8fc3e8' },
  favorite_channel: { icon: Heart, color: '#e493a0' },
  subs_gifted: { icon: HandHeart, color: '#b9a6ec' },
  twitch_age: { icon: Cake, color: '#e493a0' },
  followers: { icon: Users, color: '#8fc3e8' },
  views: { icon: Eye, color: '#aebfca' },
  account_type: { icon: Award, color: '#b9a6ec' },
  cosmetics: { icon: Palette, color: '#e493a0' },
  member_rank: { icon: Sparkles, color: '#aebfca' },
};

/** The desktop medallions' lucide icons by name (src/components/profile/accoladeIcons.ts). */
export { ACCOLADE_ICONS } from '../../components/profile/accoladeIcons';

/** The slug of art the apps ship (`streamnook-house:<slug>`, `@chat` for the
 *  chat-size cut), or null for an ordinary URL. */
export function houseSlug(url: string): string | null {
  const prefix = 'streamnook-house:';
  if (!url.startsWith(prefix)) return null;
  return url.slice(prefix.length).replace(/@chat$/, '') || null;
}
