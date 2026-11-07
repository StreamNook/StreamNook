// The Rust drops model (stores/watchDropsStore) in the shape the title bar,
// Home, the Drops page, the chat Tools menu and session resume already read
// (DropProgressStatus). A pure translation: which drop and how far were
// decided in Rust.
import type { DropChannel, DropProgressStatus } from '../types';
import type { WatchDrops } from '../stores/watchDropsStore';

/** The status that says nothing is being earned. */
export const EMPTY_DROP_STATUS = (): DropProgressStatus => ({
  active: false,
  complete: false,
  current_channel: null,
  current_campaign: null,
  current_drop: null,
  eligible_channels: [],
  last_update: new Date().toISOString(),
});

export interface ShownStatus {
  /** Null: nothing to show for this view. */
  status: DropProgressStatus | null;
  active: boolean;
  complete: boolean;
}

/** `displayName` is the watched channel's display name when the caller knows
 *  it (the model carries the login). */
export function statusFromView(view: WatchDrops, now: string, displayName?: string): ShownStatus {
  if (!view.channel_id) return { status: null, active: false, complete: false };
  const channel: DropChannel = {
    id: view.channel_id,
    name: view.login,
    display_name: displayName || view.login,
    game_name: view.game_name,
    viewer_count: 0,
    is_live: true,
    drops_enabled: true,
  };
  const lead = view.lead;
  if (lead) {
    const campaign = view.campaigns.find((c) => c.id === lead.campaign_id);
    const tier = campaign?.drops.find((d) => d.id === lead.drop_id);
    return {
      active: true,
      complete: false,
      status: {
        active: true,
        current_channel: channel,
        current_campaign: lead.campaign_id,
        current_drop: {
          campaign_id: lead.campaign_id,
          campaign_name: campaign?.name ?? '',
          drop_id: lead.drop_id,
          drop_name: lead.name,
          drop_image: lead.image || undefined,
          required_minutes: tier?.required_minutes ?? 0,
          current_minutes: tier?.current_minutes ?? 0,
          game_name: campaign?.game_name || view.game_name,
          detail: tier?.detail ?? undefined,
        },
        eligible_channels: [],
        last_update: now,
      },
    };
  }
  if (view.complete) {
    return {
      active: false,
      complete: true,
      status: {
        active: false,
        complete: true,
        current_channel: channel,
        current_campaign: view.campaigns[0]?.id ?? null,
        current_drop: null,
        eligible_channels: [],
        last_update: now,
      },
    };
  }
  return { status: null, active: false, complete: false };
}

export interface TierUpdate {
  drop_id: string;
  current_minutes: number;
  required_minutes: number;
  campaign_id: string;
}

/** One 'drops-progress-update' per watch-time tier of every campaign under way
 *  here (the Drops page's tier bars listen for these). */
export function tierUpdates(view: WatchDrops): TierUpdate[] {
  const out: TierUpdate[] = [];
  for (const campaign of view.campaigns) {
    if (!campaign.started) continue;
    for (const drop of campaign.drops) {
      if (drop.required_minutes <= 0) continue;
      out.push({
        drop_id: drop.id,
        current_minutes: drop.current_minutes,
        required_minutes: drop.required_minutes,
        campaign_id: campaign.id,
      });
    }
  }
  return out;
}
