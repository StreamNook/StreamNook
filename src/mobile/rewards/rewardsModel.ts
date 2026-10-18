// The Rewards tab's two models, built in Rust and shared with the iPhone app:
// the Drops segment (services/rewards_drops.rs, `get_rewards_drops`) and the
// badge wall (services/badge_wall.rs, `get_badge_wall`). Every line the screen
// prints is worked out there; the screen only lays it out.
import { invoke } from '@tauri-apps/api/core';

export interface CampaignChannel {
  /** The broadcaster's Twitch user id. */
  id: string;
  name: string;
}

export interface DropRow {
  id: string;
  name: string;
  image: string;
  claimed: boolean;
  done: boolean;
  /** "Claimed", "Ready", "12m left", or empty. */
  status: string;
  detail: string | null;
  /** 0 to 100; null for a drop watch time does not earn. */
  bar: number | null;
}

export interface CampaignCard {
  id: string;
  name: string;
  game_id: string;
  game_name: string;
  art: string;
  /** "2/5" in progress; empty for a campaign not started. */
  claimed_label: string;
  percent: number;
  /** "3 rewards · 2d left". */
  detail: string;
  needs_link: boolean;
  link_url: string | null;
  meta_label: string;
  channels: CampaignChannel[];
  channels_label: string;
  find_stream: { id: string; name: string; box_art_url: string } | null;
  drops: DropRow[];
}

export interface DropsGroup {
  /** `progress:<game>` or `available:<game>`. */
  key: string;
  game: string;
  art: string;
  count_label: string;
  campaigns: CampaignCard[];
}

export interface RewardsDrops {
  connected: boolean;
  query: string;
  in_progress: DropsGroup[];
  available: DropsGroup[];
  earned: { id: string; name: string; image: string; game: string | null }[];
  in_progress_empty: string;
  /** `Nothing matches “rust”.` when a search left every list empty (no
   *  sections are drawn then); empty otherwise. */
  no_match: string;
}

/** `refresh` reads Twitch again; otherwise Rust re-filters its last read by `query`. */
export function getRewardsDrops(query: string, refresh: boolean): Promise<RewardsDrops> {
  return invoke<RewardsDrops>('get_rewards_drops', { query, refresh });
}

/** A run of text, or an instant written in the viewer's own time zone. */
export interface TextPart {
  text: string | null;
  ms: number | null;
}

export interface BadgeTile {
  key: string;
  keys: string[];
  title: string;
  image: string;
  owned: boolean;
  status: 'available' | 'coming-soon' | 'expired' | null;
  word: string;
  sheet_word: string;
  prose: TextPart[];
  window: TextPart[];
  caveat: string | null;
  usage_label: string | null;
  info_url: string | null;
}

export interface EarnChip {
  kind: 'watch' | 'subscribe' | 'purchase' | 'cheer' | 'create' | 'attend' | 'other';
  ticket: boolean;
  text: string;
  hint: string;
  icon_url: string | null;
}

export interface MissingRow {
  key: string;
  title: string;
  image: string;
  chips: EarnChip[];
  icons_only: boolean;
  random_note: string | null;
  /** "Only on a, b" or "Only on 5 channels". */
  channels_note: string | null;
  time_left: string;
  /** It closes within a day: the time left is drawn as a warning. */
  urgent: boolean;
}

export interface CatchUp {
  /** "To catch up" or "To catch up, at least". */
  label: string;
  stats: { value: string; label: string }[];
  /** How the totals are counted, one line each. */
  notes: string[];
}

export interface MissingNow {
  state: 'hidden' | 'loading' | 'failed' | 'none' | 'list';
  refreshing: boolean;
  closes: string;
  catch_up: CatchUp | null;
  rows: MissingRow[];
}

export interface BadgeWall {
  loading: boolean;
  tiles: BadgeTile[];
  count_label: string;
  sync_left: number;
  missing: MissingNow;
  next_change_ms: number | null;
}

export type BadgeSort = 'newest' | 'available' | 'soon' | 'usage' | 'oldest';

export function getBadgeWall(sort: BadgeSort, force: boolean): Promise<BadgeWall> {
  return invoke<BadgeWall>('get_badge_wall', { sort, force });
}

/** Badge copy with each instant in local time: "Mar 4, 3:00 PM", the year only when not this one. */
export function blurbText(parts: TextPart[], now: number = Date.now()): string {
  const year = new Date(now).getFullYear();
  return parts
    .map((part) => {
      if (part.ms == null) return part.text ?? '';
      const date = new Date(part.ms);
      return date.toLocaleString(undefined, {
        month: 'short',
        day: 'numeric',
        ...(date.getFullYear() === year ? {} : { year: 'numeric' }),
        hour: 'numeric',
        minute: '2-digit',
      });
    })
    .join('')
    .trim();
}
