// The Rust drops model reaches the title bar in the shape it already reads:
// the lead as the current drop, an all-earned place as complete, nothing as
// null; and every started tier as one progress update.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { statusFromView, tierUpdates } from './watchDropsStatus';
import type { WatchDrops } from '../stores/watchDropsStore';

const NOW = '2026-11-06T12:00:00.000Z';
const base: WatchDrops = {
  channel_id: '111', login: 'shroud', game_id: 'g1', game_name: 'Rust', lead: null, more: 0, more_art: [],
  campaigns: [], complete: false, stalled: false, stall_episode: 0,
};
const hoodie = {
  id: 'b', instance_id: 'i-b', name: 'Hoodie', image: 'img-b', claimed: false, ready: false, pending: false,
  watched: false, lead: true, status: '30m to watch', detail: 'Day 1 of 2', bar: 75, current_minutes: 90,
  required_minutes: 120, claim_label: 'Claim',
};
const withLead: WatchDrops = {
  ...base,
  more: 1,
  lead: {
    campaign_id: 'c1', drop_id: 'b', instance_id: 'i-b', name: 'Hoodie', image: 'img-b', trailing: '30m to watch',
    status: '30m to watch', line: 'Rust Drops · 2 of 3', percent: 75, complete: false, claim_label: 'Claim',
    spoken: 'Hoodie, 30 minutes left',
  },
  campaigns: [
    {
      id: 'c1', name: 'Rust Drops', game_name: 'Rust', art: '', box_art: '', ends: '3d left', claimed_label: '1 of 3 claimed',
      started: true, cumulative_minutes: 90,
      drops: [{ ...hoodie, id: 'a', lead: false, claimed: true, current_minutes: 60, required_minutes: 60 }, hoodie,
        { ...hoodie, id: 'zero', lead: false, required_minutes: 0, current_minutes: 0 }],
    },
    {
      id: 'c2', name: 'Later', game_name: 'Rust', art: '', box_art: '', ends: '', claimed_label: '', started: false,
      cumulative_minutes: 0, drops: [{ ...hoodie, id: 'n', lead: false, current_minutes: 0 }],
    },
  ],
};

describe('statusFromView', () => {
  it('shows nothing for the empty view', () => {
    expect(statusFromView({ ...base, channel_id: '' }, NOW)).toEqual({ status: null, active: false, complete: false });
    expect(statusFromView(base, NOW).status).toBeNull();
  });

  it('makes the lead the current drop with its tier minutes', () => {
    const shown = statusFromView(withLead, NOW, 'Shroud');
    expect(shown.active).toBe(true);
    expect(shown.status?.current_channel?.display_name).toBe('Shroud');
    expect(shown.status?.current_drop).toMatchObject({
      campaign_id: 'c1', campaign_name: 'Rust Drops', drop_id: 'b', drop_name: 'Hoodie', drop_image: 'img-b',
      current_minutes: 90, required_minutes: 120, game_name: 'Rust', detail: 'Day 1 of 2',
    });
    expect(statusFromView(withLead, NOW).status?.current_channel?.display_name).toBe('shroud');
  });

  it('reads a place with everything earned as complete', () => {
    const shown = statusFromView({ ...base, complete: true, campaigns: withLead.campaigns }, NOW);
    expect(shown).toMatchObject({ active: false, complete: true });
    expect(shown.status).toMatchObject({ complete: true, current_drop: null, current_campaign: 'c1' });
  });
});

describe('tierUpdates', () => {
  it('sends every watch-time tier of a started campaign', () => {
    expect(tierUpdates(withLead)).toEqual([
      { drop_id: 'a', current_minutes: 60, required_minutes: 60, campaign_id: 'c1' },
      { drop_id: 'b', current_minutes: 90, required_minutes: 120, campaign_id: 'c1' },
    ]);
  });
});

// The old TypeScript picker and its 30 s poll are gone: the controller only
// translates, and acts on a stall once per Rust episode.
describe('DropProgressController', () => {
  const src = readFileSync(resolve(__dirname, '../components/plugins/DropProgressController.tsx'), 'utf8');
  it('has no picker or poll of its own', () => {
    // (`get_active_drop_campaigns` stays: the provider half still resolves reward names with it.)
    expect(src).not.toMatch(/pickCurrentDrop|setInterval|get_drops_inventory|get_drop_progress/);
    expect(src).toContain('useWatchDrops(!externalDropsProvider)');
  });
  it('acts on a stall only after claiming the episode', () => {
    expect(src).toMatch(/invoke<boolean>\('ack_drop_stall', \{ episode: stallEpisode \}\)[\s\S]{0,80}if \(mine\) await handleFarmedStreamStall\(\)/);
  });
});
