// The Pokémon evolution lines above the badge wall. Twitch only counts watching
// toward the equipped line, so this is where one is picked. The board comes
// ready from Rust (`get_evolution_board`, services/evolution.rs), the same one
// the desktop Badges overlay and the iPhone draw; this lays it out at phone
// density and sends the equip or the claim.
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

// The last board this session, so returning to the tab paints at once.
let lastBoard: EvolutionBoard | null = null;
// How tall the section was the last time it had lines: while the board is on
// its way it holds that much room, so the missing list under it never jumps
// down. 0 = it had nothing to show.
const HEIGHT_KEY = 'sn.evolution.mobile-height';

function rememberHeight(px: number) {
  try {
    localStorage.setItem(HEIGHT_KEY, String(Math.round(px)));
  } catch {
    // Storage unavailable; the section just appears without a placeholder.
  }
}

function lastHeight(): number {
  try {
    return Number(localStorage.getItem(HEIGHT_KEY)) || 0;
  } catch {
    return 0;
  }
}

/** Days done as filled dots, the day under way (on the equipped line) as an
 *  accent ring, the rest as rings. Past five days, the count instead. */
const DayDots = ({ done, total, today }: { done: number; total: number; today: boolean }) => {
  if (total > 5) {
    return (
      <span className="text-[11px] font-medium tabular-nums text-textSecondary" aria-hidden>
        {done}/{total}
      </span>
    );
  }
  return (
    <span className="flex items-center gap-1" aria-hidden>
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

const LaneRow = ({
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
  const pickable = lane.can_equip && !equipped && !busy;
  const ready = target.ready_to_claim && target.instance_id;
  const detail = laneDetail(lane, equipped);
  const showDays = !done && !blocked && (equipped || target.days_done > 0);

  return (
    <div
      className={`glass-panel relative ${equipped ? 'ring-1 ring-accent/50' : ''} [&:has(>.sn-evo-pick:active)]:opacity-80`}
    >
      {/* The whole row picks the line; the claim button sits above it as its
          own button, since nesting one button in another is invalid. */}
      <button
        type="button"
        disabled={!pickable}
        onClick={onEquip}
        aria-pressed={lane.can_equip ? equipped : undefined}
        aria-label={`${target.title}, ${detail}`}
        className="sn-evo-pick absolute inset-0 w-full rounded-[inherit] disabled:cursor-default"
      />
      <div className="relative pointer-events-none flex items-center gap-3 px-3 py-2">
        <img
          src={target.image_url}
          alt=""
          className={`w-9 h-9 object-contain shrink-0 ${blocked ? 'grayscale opacity-50' : done ? 'opacity-80' : ''}`}
          draggable={false}
        />
        <div className="flex-1 min-w-0 h-9 flex flex-col justify-center">
          <span className={`text-[13px] leading-4 font-medium truncate ${blocked ? 'text-textSecondary' : 'text-textPrimary'}`}>
            {target.title}
          </span>
          <span className={`mt-0.5 text-[11px] truncate ${equipped ? 'font-medium text-accent' : 'text-textMuted'}`}>
            {detail}
          </span>
        </div>
        {ready ? (
          <button
            type="button"
            onClick={() => onClaim(target.group_id, target.instance_id!)}
            className="pointer-events-auto shrink-0 -my-2 py-2 px-1 active:opacity-70 transition-opacity"
          >
            <span className="glaze-inset glaze-chip bg-white/[0.10] inline-flex items-center h-[24px] px-2.5 text-[11.5px] font-medium text-success">
              {target.claim_label}
            </span>
          </button>
        ) : done ? (
          <Check size={14} className="shrink-0 self-start mt-[3px] text-success" aria-hidden />
        ) : showDays ? (
          <span className="shrink-0 self-start mt-[6px]">
            <DayDots done={target.days_done} total={target.total_days} today={equipped} />
          </span>
        ) : null}
      </div>
    </div>
  );
};

export function MobileEvolution() {
  const [board, setBoard] = useState<EvolutionBoard | null>(lastBoard);
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
        .catch((err) => Logger.warn('[MobileEvolution] Evolution board unavailable:', err)),
    [],
  );

  useEffect(() => {
    // Forced on visit: the line may have been changed on twitch.tv since.
    void load(true);
    const unlisten = listen('badge-standing-changed', () => void load(false));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [load]);

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
      Logger.warn('[MobileEvolution] Claim failed:', err);
      useAppStore.getState().addToast("Couldn't claim that on Twitch. Try again in a moment.", 'error');
    }
  };

  if (!board) {
    return placeholderPx > 0 ? (
      <section className="pb-3" aria-hidden style={{ height: placeholderPx }}>
        <header className="pb-2.5">
          <span className="missing-now-title">Evolution</span>
          <p className="h-[17px]" />
        </header>
        <div className="h-[52px] rounded-xl glass-panel animate-pulse" />
      </section>
    ) : null;
  }
  if (board.tracks.length === 0) return null;

  return (
    <div ref={rootRef}>
      {board.tracks.map((track) => {
        const active = shownLane(track, pending);
        const progress = active && active.current !== null ? active.line : null;
        return (
          <section key={track.track_id} className="pb-3">
            <header className="pb-2.5">
              <span className="missing-now-title">Evolution</span>
              <p className="text-[11.5px] text-textMuted truncate">
                {progress ?? (active ? 'Evolving' : 'Pick a Pokémon to evolve · one at a time')}
                {' · ends '}
                <span className="font-medium text-textSecondary">{endsLabel(track.ends_ms)}</span>
              </p>
            </header>
            <div className="flex flex-col gap-1.5">
              {track.lanes.map((lane) => (
                <LaneRow
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
}
