import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Check } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import {
  endsLabel,
  equipEvolutionLane,
  getEvolutionBoard,
  laneDetail,
  laneTarget,
  shownLane,
  type EvolutionBoard,
  type EvolutionLane,
  type EvolutionTrack,
} from '../../services/evolution';
import { Tooltip } from '../ui/Tooltip';

// Same height as a Missing card, so the two strips share one rhythm.
const TILE_HEIGHT_PX = 64;
// Remembered per viewer: how tall the strip was the last time it had lines.
// While the board is on its way the strip holds that much room, so nothing
// below it moves down when it arrives. 0 = it had nothing to show.
const HEIGHT_KEY = 'sn.evolution.height';
// The last board this session, so a reopen paints at once.
let lastBoard: EvolutionBoard | null = null;

function rememberHeight(px: number) {
  try {
    localStorage.setItem(HEIGHT_KEY, String(Math.round(px)));
  } catch {
    // Storage unavailable; the strip just appears without a placeholder.
  }
}

function lastHeight(): number {
  try {
    return Number(localStorage.getItem(HEIGHT_KEY)) || 0;
  } catch {
    return 0;
  }
}

/** The Pokémon evolution lines, pinned above "Missing, earnable now" in the
 *  Badges overlay. Twitch only counts watching toward the equipped line, so this
 *  is where one is picked. Everything comes ready from Rust
 *  (`get_evolution_board`); this lays it out and sends the equip or the claim. */
export const EvolutionPicker = () => {
  const [board, setBoard] = useState<EvolutionBoard | null>(lastBoard);
  // The lane being equipped, shown as equipped until Twitch answers.
  const [pending, setPending] = useState<{ track: string; lane: string } | null>(null);
  const [placeholderPx] = useState(lastHeight);
  const rootRef = useRef<HTMLDivElement>(null);

  const load = useCallback(
    (force: boolean) =>
      getEvolutionBoard(force)
        .then((next) => {
          lastBoard = next;
          setBoard(next);
          if (next.tracks.length === 0) rememberHeight(0);
        })
        .catch((err) => Logger.warn('[EvolutionPicker] Evolution board unavailable:', err)),
    [],
  );

  useEffect(() => {
    // Forced on open: the line may have been changed on twitch.tv since.
    void load(true);
    const unlisten = listen('badge-standing-changed', () => void load(false));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [load]);

  // The room the next open holds while it loads: this strip's real height.
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => rememberHeight(el.offsetHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, [board]);

  const equip = async (track: EvolutionTrack, lane: EvolutionLane) => {
    if (pending || lane.equipped || !lane.can_equip) return;
    setPending({ track: track.track_id, lane: lane.lane_id });
    try {
      await equipEvolutionLane(track.track_id, lane.lane_id);
      // Held as equipped until the board that says so has arrived.
      await load(false);
    } catch (err) {
      useAppStore.getState().addToast(typeof err === 'string' ? err : "Twitch didn't accept that. Try again in a moment.", 'error');
    } finally {
      setPending(null);
    }
  };

  const claim = async (groupId: string, instanceId: string) => {
    try {
      await invoke('claim_drop', { dropId: groupId, dropInstanceId: instanceId });
      await load(false);
    } catch (err) {
      Logger.warn('[EvolutionPicker] Claim failed:', err);
      useAppStore.getState().addToast("Couldn't claim that on Twitch. Try again in a moment.", 'error');
    }
  };

  if (!board) {
    return placeholderPx > 0 ? (
      <section className="mb-6" aria-hidden style={{ height: placeholderPx }}>
        <h3 className="missing-now-title">Evolution</h3>
        <p className="mt-0.5 h-[17px]" />
        <div className="missing-now-rule" />
        <div
          className="rounded-xl bg-white/[0.03] border border-white/[0.05] animate-pulse"
          style={{ height: TILE_HEIGHT_PX }}
        />
      </section>
    ) : null;
  }
  if (board.tracks.length === 0) return null;

  return (
    <div ref={rootRef}>
      {board.tracks.map((track) => {
        // The equipped line's progress gets the full width here; its tile
        // only says it is the one evolving.
        const active = shownLane(track, pending);
        const progress = active && active.current !== null ? active.line : null;
        return (
          <section key={track.track_id} className="mb-6">
            <h3 className="missing-now-title">Evolution</h3>
            <p className="mt-0.5 text-[11.5px] text-textMuted truncate">
              {progress ?? (active ? 'Evolving' : 'Pick a Pokémon to evolve · one at a time')}
              {' · ends '}
              <span className="font-medium text-textSecondary">{endsLabel(track.ends_ms)}</span>
            </p>
            <div aria-hidden className="missing-now-rule" />
            <div
              className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3"
              style={{ gridAutoRows: TILE_HEIGHT_PX }}
            >
              {track.lanes.map((lane) => (
                <LaneTile
                  key={lane.lane_id}
                  lane={lane}
                  equipped={active?.lane_id === lane.lane_id}
                  busy={pending !== null}
                  onEquip={() => void equip(track, lane)}
                  onClaim={(groupId, instanceId) => void claim(groupId, instanceId)}
                />
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
};

/** Days done as filled dots, the day under way (on the equipped line) as an
 *  accent ring, the rest as rings: a step count, never a "more" menu. Past
 *  five days the dots would crowd the title out, so it says the count. */
const DayDots = ({ done, total, today }: { done: number; total: number; today: boolean }) => {
  const label = `Day ${Math.min(done, total)} of ${total} done`;
  if (total > 5) {
    return (
      <span className="text-[11px] font-medium tabular-nums text-textSecondary" aria-label={label}>
        {done}/{total}
      </span>
    );
  }
  return (
    <span className="flex items-center gap-1" aria-label={label}>
      {Array.from({ length: total }, (_, i) => (
        <span
          key={i}
          className={`w-1.5 h-1.5 rounded-full ${
            i < done
              ? 'bg-accent'
              : today && i === done
                ? 'ring-1 ring-inset ring-accent'
                : 'ring-1 ring-inset ring-white/[0.35]'
          }`}
        />
      ))}
    </span>
  );
};

const LaneTile = ({
  lane,
  equipped,
  busy,
  onEquip,
  onClaim,
}: {
  lane: EvolutionLane;
  equipped: boolean;
  busy: boolean;
  onEquip: () => void;
  onClaim: (groupId: string, instanceId: string) => void;
}) => {
  const target = laneTarget(lane);
  if (!target) return null;
  const done = lane.current === null;
  const blocked = !done && lane.blocked_by !== null;
  const ready = target.ready_to_claim && target.instance_id;
  const detail = laneDetail(lane, equipped);
  // Days only where they say something: the line evolving, or one with days in.
  const showDays = !done && !blocked && (equipped || target.days_done > 0);

  const pickable = lane.can_equip && !equipped && !busy;
  const tone = equipped
    ? 'bg-white/[0.07] border-accent/50 ring-1 ring-accent/30'
    : blocked
      ? 'bg-white/[0.02] border-white/[0.04]'
      : done
        ? 'bg-white/[0.04] border-white/[0.06]'
        : 'bg-white/[0.04] hover:bg-white/[0.07] border-white/[0.06]';

  return (
    <Tooltip content={pickable ? `Evolve ${lane.from ?? 'this badge'} into ${target.title}` : undefined} delay={500}>
      <div
        role={pickable ? 'button' : undefined}
        tabIndex={pickable ? 0 : undefined}
        aria-pressed={lane.can_equip ? equipped : undefined}
        aria-label={pickable ? `Evolve ${lane.from ?? target.title} into ${target.title}` : undefined}
        onClick={pickable ? onEquip : undefined}
        onKeyDown={
          pickable
            ? (e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onEquip();
                }
              }
            : undefined
        }
        className={`group min-w-0 h-full flex items-center gap-3 rounded-xl border px-3 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${tone} ${
          pickable ? 'cursor-pointer' : ''
        }`}
      >
        <img
          src={target.image_url}
          alt=""
          className={`shrink-0 w-10 h-10 object-contain ${blocked ? 'grayscale opacity-50' : done ? 'opacity-80' : ''}`}
          loading="lazy"
          draggable={false}
        />
        <div className="flex-1 min-w-0">
          <div
            className={`text-[13px] font-medium truncate transition-colors ${
              blocked ? 'text-textSecondary' : 'text-textPrimary'
            } ${pickable ? 'group-hover:text-accent' : ''}`}
          >
            {target.title}
          </div>
          <Tooltip content={detail}>
            <div className={`mt-1 text-[11px] truncate ${equipped ? 'font-medium text-accent' : 'text-textMuted'}`}>
              {detail}
            </div>
          </Tooltip>
        </div>
        {ready ? (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onClaim(target.group_id, target.instance_id!);
            }}
            className="shrink-0 px-2.5 py-1 rounded-lg glass-button text-[11px] font-medium text-success"
          >
            {target.claim_label}
          </button>
        ) : done ? (
          // On the title's cap height, like the dots.
          <Check size={14} className="shrink-0 self-start mt-[14px] text-success" aria-label="All evolved" />
        ) : showDays ? (
          <span className="shrink-0 self-start mt-[18px]">
            <DayDots done={target.days_done} total={target.total_days} today={equipped} />
          </span>
        ) : null}
      </div>
    </Tooltip>
  );
};
