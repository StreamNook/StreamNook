// Pokémon badge evolution, built in Rust (services/evolution.rs): each evolution
// line, which one is equipped on Twitch, and each stage's progress. This module
// only fetches it and sends the equip; the lane grouping, the base-badge check
// and the progress words all live in Rust.
import { invoke } from '@tauri-apps/api/core';

export interface EvolutionBadge {
  title: string;
  image_url: string;
  owned: boolean;
}

export interface EvolutionStage {
  group_id: string;
  title: string;
  image_url: string;
  earned: boolean;
  days_done: number;
  total_days: number;
  ready_to_claim: boolean;
  /** What `claim_drop` sends when the stage is ready. */
  instance_id: string | null;
  claim_label: string;
}

export interface EvolutionLane {
  lane_id: string;
  /** The badge the line starts from (Gastly), when known. */
  base: EvolutionBadge | null;
  stages: EvolutionStage[];
  /** The stage being worked on; null once the line is done. */
  current: number | null;
  equipped: boolean;
  can_equip: boolean;
  /** The badge to hold first, when it is known not to be held. */
  blocked_by: string | null;
  /** What the current stage evolves from. */
  from: string | null;
  /** The current stage's progress in words. */
  line: string | null;
}

export interface EvolutionTrack {
  track_id: string;
  campaign_id: string;
  campaign_name: string;
  ends_ms: number;
  equipped_lane: string | null;
  lanes: EvolutionLane[];
}

export interface EvolutionBoard {
  /** Empty when there is nothing to evolve, or no Drops sign-in. */
  tracks: EvolutionTrack[];
}

export function getEvolutionBoard(force = false): Promise<EvolutionBoard> {
  return invoke<EvolutionBoard>('get_evolution_board', { force });
}

/** Rejects with words to show when Twitch refuses. */
export function equipEvolutionLane(track: string, lane: string): Promise<void> {
  return invoke<void>('equip_evolution_lane', { track, lane });
}

/** The stage a lane is on, or its last once the line is done. */
export function laneTarget(lane: EvolutionLane): EvolutionStage | null {
  if (lane.stages.length === 0) return null;
  return lane.stages[lane.current ?? lane.stages.length - 1];
}

/** A lane tile's one line of words: what it is doing right now. */
export function laneDetail(lane: EvolutionLane, equipped: boolean): string {
  if (lane.current === null) return 'All evolved';
  if (lane.blocked_by) return `Needs ${lane.blocked_by}`;
  if (equipped) return 'Evolving';
  const target = laneTarget(lane);
  return lane.from ? `From ${lane.from}` : `${target?.total_days ?? 3} days to evolve`;
}

/** The lane shown as equipped: the one being equipped while Twitch answers. */
export function shownLane(
  track: EvolutionTrack,
  pending: { track: string; lane: string } | null,
): EvolutionLane | undefined {
  return track.lanes.find((l) =>
    pending ? pending.track === track.track_id && pending.lane === l.lane_id : l.equipped,
  );
}

export const endsLabel = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
