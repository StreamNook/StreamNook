// The drops you can earn on the channel this account is earning on, as Rust
// keeps them (services/watch_drops.rs). Nothing here decides anything: the
// chat bar and the title-bar adapter read this one copy per window, and only
// Rust writes it. A value is taken only when its rev is newer than the one
// held, so a late read can never put back an older answer than an event
// already brought.
import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';
import type { DropProgress } from '../types';

export const WATCH_DROPS_CHANGED = 'watch-drops://changed';

/** Rust `watch_drop::WatchDrop`: the lead reward, the one the bar shows. */
export interface WatchDropLead {
  campaign_id: string;
  drop_id: string;
  instance_id: string | null;
  name: string;
  image: string;
  trailing: string;
  status: string;
  line: string;
  percent: number;
  complete: boolean;
  claim_label: string;
  spoken: string;
}

/** Rust `watch_drops::HereDrop`: one reward tier. */
export interface HereDrop {
  id: string;
  instance_id: string | null;
  name: string;
  image: string;
  claimed: boolean;
  ready: boolean;
  pending: boolean;
  /** Minutes in, but Twitch has not granted the claim; the auto-claim tries again later. */
  watched: boolean;
  lead: boolean;
  status: string;
  detail: string | null;
  bar: number | null;
  current_minutes: number;
  required_minutes: number;
  claim_label: string;
}

/** Rust `watch_drops::HereCampaign`. */
export interface HereCampaign {
  id: string;
  name: string;
  game_name: string;
  art: string;
  /** The game's box art, sized; '' for a campaign with no category. */
  box_art: string;
  ends: string;
  claimed_label: string;
  started: boolean;
  cumulative_minutes: number;
  drops: HereDrop[];
}

/** Rust `watch_drops::WatchDrops`. An empty `channel_id` is the empty view. */
export interface WatchDrops {
  channel_id: string;
  login: string;
  game_id: string;
  game_name: string;
  lead: WatchDropLead | null;
  more: number;
  /** Pictures of the first two of those others, stacked on the bar. */
  more_art: string[];
  campaigns: HereCampaign[];
  complete: boolean;
  stalled: boolean;
  stall_episode: number;
}

export interface WatchDropsSnapshot {
  rev: number;
  view: WatchDrops;
}

let current: WatchDropsSnapshot | null = null;
const changeListeners = new Set<() => void>();
let readers = 0;
// Bumped whenever the last reader leaves, so a listen still resolving for a
// reader that already left lets go of itself instead of leaking.
let generation = 0;
let unlisten: (() => void) | null = null;

/** Takes `next` when it is newer than what is held. True when it was taken. */
export function applyWatchDrops(next: WatchDropsSnapshot | null | undefined): boolean {
  if (!next || !next.view) return false;
  if (current && next.rev <= current.rev) return false;
  current = next;
  changeListeners.forEach((l) => l());
  return true;
}

export const getWatchDrops = (): WatchDropsSnapshot | null => current;
const getNothing = (): WatchDropsSnapshot | null => null;

/** The first reader registers the listener, then reads the current value, so
 *  nothing that changes in between is lost (the read can only lose to a newer
 *  event, never overwrite one). The last reader unregisters. */
function retain(): () => void {
  readers += 1;
  if (readers === 1) {
    const mine = generation;
    void (async () => {
      try {
        const un = await listen<WatchDropsSnapshot>(WATCH_DROPS_CHANGED, (e) => {
          applyWatchDrops(e.payload);
        });
        if (mine !== generation) {
          un();
          return;
        }
        unlisten = un;
        applyWatchDrops(await invoke<WatchDropsSnapshot>('get_watch_drops'));
      } catch (err) {
        Logger.warn('[WatchDrops] could not follow the drops model:', err);
      }
    })();
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    readers -= 1;
    if (readers > 0) return;
    generation += 1;
    unlisten?.();
    unlisten = null;
    // Nobody follows the model any more, so what is held would only go stale
    // (the Drops page lays it over fresher minutes). The next reader reads anew.
    current = null;
  };
}

function subscribe(onChange: () => void): () => void {
  changeListeners.add(onChange);
  const release = retain();
  return () => {
    changeListeners.delete(onChange);
    release();
  };
}
const subscribeNothing = (): (() => void) => () => {};

/** The subscribe function for `enabled`; off subscribes to nothing at all, so a
 *  chat with the setting off holds no listener. */
export const subscribeFor = (enabled: boolean) => (enabled ? subscribe : subscribeNothing);

/** The drops model while `enabled`, else null. */
export function useWatchDrops(enabled = true): WatchDropsSnapshot | null {
  const getter = enabled ? getWatchDrops : getNothing;
  return useSyncExternalStore(subscribeFor(enabled), getter, getter);
}

/** For tests: how many readers hold the listener. */
export const watchDropsReaders = (): number => readers;

/** For tests only. */
export function resetWatchDropsForTest(): void {
  current = null;
  changeListeners.clear();
  readers = 0;
  generation += 1;
  unlisten = null;
}

/** The Drops page's per-drop progress with the model's tier minutes laid over
 *  it, for every campaign earning here. The page's own list comes from the
 *  monitoring loop's per-drop counters, which zero a claimed tier; the model
 *  carries the campaign clock, so the page and the bar show the same minutes. */
export function overlayWatchTiers(
  progress: DropProgress[],
  view: WatchDrops | null | undefined,
): DropProgress[] {
  if (!view || !view.channel_id) return progress;
  const tiers = new Map<string, { campaignId: string; drop: HereDrop }>();
  for (const campaign of view.campaigns) {
    if (!campaign.started) continue;
    for (const drop of campaign.drops) {
      if (drop.required_minutes > 0) tiers.set(drop.id, { campaignId: campaign.id, drop });
    }
  }
  if (tiers.size === 0) return progress;
  const out = progress.map((p) => {
    const tier = tiers.get(p.drop_id);
    if (!tier) return p;
    tiers.delete(p.drop_id);
    return {
      ...p,
      current_minutes_watched: tier.drop.current_minutes,
      required_minutes_watched: tier.drop.required_minutes,
      is_claimed: p.is_claimed || tier.drop.claimed,
    };
  });
  for (const [dropId, tier] of tiers) {
    out.push({
      campaign_id: tier.campaignId,
      drop_id: dropId,
      current_minutes_watched: tier.drop.current_minutes,
      required_minutes_watched: tier.drop.required_minutes,
      is_claimed: tier.drop.claimed,
      last_updated: '',
    });
  }
  return out;
}
