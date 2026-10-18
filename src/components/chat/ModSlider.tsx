import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react';
import { Clock, Prohibit, Trash, X } from 'phosphor-react';

import { useAppStore } from '../../stores/AppStore';

import {
  holdChatRow,
  moderateWithToast,
  useModSliderSteps,
  useSliderConfig,
  type ModAction,
  type ModRequest,
  type ModStep,
} from '../../services/moderation';

/**
 * The slider on a chat row's left edge, for moderators: drag right through
 * the timeout steps to Ban, let go to apply; pull back to the start or press
 * Escape to cancel.
 *
 * Presentation only. The steps are Rust's (`get_mod_slider_steps`: Delete, the
 * timeouts the platform will really apply, Ban, mildest first) and the action
 * goes through `moderate_chatter`.
 * The row moves by CSS variables written straight onto its element, so a drag
 * never re-renders the row; this component renders again only when the step
 * changes. Everything that follows the pointer (the row, the fill, the label,
 * the ruler and the edge shadow) moves by transform alone, so a drag costs no
 * layout and no repaint. Mounted only on a hovered row, so idle rows carry
 * none of it.
 */

/** Pull this far before the first step; less is the cancel zone. Wide
 *  enough that the first step's label fits the track once it is armed. */
const DEAD_ZONE_PX = 72;
/** Room per step, scaled to the row's width between these bounds. */
const STEP_MIN_PX = 16;
const STEP_MAX_PX = 52;
/** Share of the row the steps spread over, by the Length setting. */
const LENGTH_SHARE = { short: 0.5, medium: 0.8, long: 1 } as const;
/** Ban sits this many extra step-lengths past the last timeout, so it is
 *  never reached by a pull that only overshot the longest timeout. */
const BAN_GAP_STEPS = 1;
/** Past the last step the row follows at this fraction, then stops. */
const OVERPULL = 0.2;
const RETURN_MS = 220;
/** Ban arms only after the pull rests this long in its zone, the track
 *  filling red meanwhile. Until then the track shows the step before it and
 *  a release applies exactly that, so a flick past the end never bans. */
const BAN_DWELL_MS = 350;

interface DragState {
  pointerId: number;
  startX: number;
  row: HTMLElement;
  /** The row's width at the press, for the fill's scale. */
  width: number;
  spacing: number;
  engaged: boolean;
  step: number;
  /** Running while the pull rests in Ban's zone before Ban arms. */
  banTimer: number | null;
}

interface ModSliderProps {
  provider: string;
  /** The chatter and channel; the slider adds the action. */
  target: Omit<ModRequest, 'action'>;
  /** The message, for the Delete step (left out without one). */
  messageId?: string;
  /** Settings preview: report the step instead of acting on it. */
  onApply?: (step: ModStep) => void;
  /** Show the grip without a hover (the settings preview). */
  hint?: boolean;
}

/** Capture can throw when the pointer is already gone (released between the
 *  press and this call); the drag then simply ends on the next pointer up. */
function capturePointer(el: HTMLElement, pointerId: number) {
  try {
    el.setPointerCapture(pointerId);
  } catch {
    // Nothing to capture.
  }
}

/** Delete is a calm slate, timeouts warm from amber toward red as they
 *  lengthen, Ban is red. */
function stepTint(steps: ModStep[], index: number): string {
  const step = steps[index];
  if (step.action === 'ban') return 'var(--color-error)';
  if (step.action === 'delete') return 'color-mix(in srgb, var(--color-text-secondary) 55%, #334155)';
  const timeouts = steps.filter((s) => s.action === 'timeout');
  const at = timeouts.indexOf(step);
  const toward = timeouts.length > 1 ? Math.round((at / (timeouts.length - 1)) * 65) : 0;
  return `color-mix(in srgb, var(--color-error) ${toward}%, var(--color-warning))`;
}

/** Step-lengths the slider spans: one per step, plus the gap before Ban. */
function stepUnits(steps: ModStep[]): number {
  const endsInBan = steps[steps.length - 1]?.action === 'ban';
  return steps.length + (endsInBan ? BAN_GAP_STEPS : 0);
}

/** The step under a pull of `slide` px, or -1 inside the cancel zone. The
 *  gap before Ban keeps showing the step before it (or nothing, when Ban is
 *  the only step). */
function stepAt(steps: ModStep[], slide: number, spacing: number): number {
  if (slide < DEAD_ZONE_PX) return -1;
  const unit = Math.floor((slide - DEAD_ZONE_PX) / spacing);
  const last = steps.length - 1;
  if (steps[last]?.action !== 'ban') return Math.min(last, unit);
  if (unit >= last + BAN_GAP_STEPS) return last;
  return Math.min(unit, last - 1);
}

/** Where each step starts, in px from the row's left edge, for the ruler. */
function rulerFor(steps: ModStep[], spacing: number): number[] {
  const last = steps.length - 1;
  return steps.map((s, i) =>
    DEAD_ZONE_PX + (s.action === 'ban' && i === last ? i + BAN_GAP_STEPS : i) * spacing,
  );
}

function actionFor(step: ModStep, messageId: string | undefined): ModAction | null {
  if (step.action === 'delete') return messageId ? { kind: 'delete', messageId } : null;
  if (step.action === 'ban') return { kind: 'ban' };
  return step.seconds != null ? { kind: 'timeout', seconds: step.seconds } : null;
}

export function ModSlider({ provider, target, messageId, onApply, hint = false }: ModSliderProps) {
  const allSteps = useModSliderSteps(provider, useSliderConfig());
  const length = useAppStore((s) => s.settings.chat_design?.mod_slider_length) ?? 'medium';
  const share = LENGTH_SHARE[length] ?? LENGTH_SHARE.medium;
  const steps = messageId ? allSteps : allSteps?.filter((s) => s.action !== 'delete') ?? null;
  // null = idle, -1 = pulled but inside the cancel zone, 0+ = the step.
  const [step, setStep] = useState<number | null>(null);
  // True while the pull rests in Ban's zone and Ban is not armed yet.
  const [arming, setArming] = useState(false);
  // Where each step starts, set once per drag when it engages.
  const [ruler, setRuler] = useState<number[] | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const returnTimerRef = useRef<number | null>(null);
  const keyListenerRef = useRef<((e: KeyboardEvent) => void) | null>(null);

  const settle = (row: HTMLElement) => {
    row.classList.remove('sn-mod-sliding', 'sn-mod-returning');
    row.style.removeProperty('--mod-slide');
    row.style.removeProperty('--mod-ratio');
  };

  const finish = (apply: boolean) => {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    if (drag.banTimer != null) window.clearTimeout(drag.banTimer);
    setArming(false);
    if (keyListenerRef.current) {
      window.removeEventListener('keydown', keyListenerRef.current, true);
      keyListenerRef.current = null;
    }
    const { row, engaged, step: chosen } = drag;
    holdChatRow(row, false);
    if (!engaged) {
      setStep(null);
      return;
    }
    // Spring back, then clear: the track collapses with the row.
    row.classList.add('sn-mod-returning');
    row.style.setProperty('--mod-slide', '0px');
    row.style.setProperty('--mod-ratio', '0');
    returnTimerRef.current = window.setTimeout(() => {
      returnTimerRef.current = null;
      settle(row);
      setStep(null);
      setRuler(null);
    }, RETURN_MS);

    const picked = apply && chosen >= 0 ? steps?.[chosen] : undefined;
    if (!picked) return;
    if (onApply) {
      onApply(picked);
      return;
    }
    const action = actionFor(picked, messageId);
    if (!action) return;
    void moderateWithToast({ ...target, action, useSavedReason: action.kind !== 'delete' });
  };

  // A row that leaves the list mid-drag cancels; nothing is ever applied on
  // unmount.
  useEffect(
    () => () => {
      if (returnTimerRef.current != null) window.clearTimeout(returnTimerRef.current);
      if (keyListenerRef.current) window.removeEventListener('keydown', keyListenerRef.current, true);
      const drag = dragRef.current;
      if (drag) {
        dragRef.current = null;
        if (drag.banTimer != null) window.clearTimeout(drag.banTimer);
        settle(drag.row);
      }
    },
    [],
  );

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !steps || steps.length === 0 || dragRef.current) return;
    const row = e.currentTarget.parentElement;
    if (!row) return;
    // No text selection, no row pickup: this press belongs to the slider.
    e.preventDefault();
    e.stopPropagation();
    if (returnTimerRef.current != null) {
      window.clearTimeout(returnTimerRef.current);
      returnTimerRef.current = null;
      settle(row);
    }
    capturePointer(e.currentTarget, e.pointerId);
    // Hold the list from the press, not the first movement: in a fast chat
    // the row would otherwise move before the drag even starts.
    holdChatRow(row, true);
    const width = row.getBoundingClientRect().width;
    const usable = width * share - DEAD_ZONE_PX;
    const spacing = Math.min(STEP_MAX_PX, Math.max(STEP_MIN_PX, usable / stepUnits(steps)));
    dragRef.current = { pointerId: e.pointerId, startX: e.clientX, row, width, spacing, engaged: false, step: -1, banTimer: null };
    const onKey = (ke: KeyboardEvent) => {
      if (ke.key !== 'Escape') return;
      ke.preventDefault();
      ke.stopPropagation();
      finish(false);
    };
    keyListenerRef.current = onKey;
    window.addEventListener('keydown', onKey, true);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || e.pointerId !== drag.pointerId || !steps) return;
    const dx = e.clientX - drag.startX;
    if (!drag.engaged) {
      if (dx < 4) return;
      drag.engaged = true;
      drag.row.classList.add('sn-mod-sliding');
      setRuler(rulerFor(steps, drag.spacing));
      // The track (and its Cancel) shows from the first pixel, not only once
      // a step arms.
      setStep(-1);
    }
    const full = DEAD_ZONE_PX + stepUnits(steps) * drag.spacing;
    const pulled = Math.max(0, dx);
    const slide = pulled <= full ? pulled : full + Math.min((pulled - full) * OVERPULL, drag.spacing);
    drag.row.style.setProperty('--mod-slide', `${slide}px`);
    drag.row.style.setProperty('--mod-ratio', String(drag.width > 0 ? slide / drag.width : 0));
    let next = stepAt(steps, slide, drag.spacing);
    const inBanZone = next >= 0 && steps[next].action === 'ban';
    if (inBanZone && drag.step !== next) {
      // Not armed yet: show the step before Ban while the dwell runs.
      if (drag.banTimer == null) {
        const ban = next;
        drag.banTimer = window.setTimeout(() => {
          if (dragRef.current !== drag) return;
          drag.banTimer = null;
          drag.step = ban;
          setArming(false);
          setStep(ban);
        }, BAN_DWELL_MS);
        setArming(true);
      }
      next -= 1;
    } else if (!inBanZone && drag.banTimer != null) {
      window.clearTimeout(drag.banTimer);
      drag.banTimer = null;
      setArming(false);
    }
    if (next !== drag.step) {
      drag.step = next;
      setStep(next);
    }
  };

  const onPointerEnd = (e: ReactPointerEvent<HTMLDivElement>, apply: boolean) => {
    const drag = dragRef.current;
    if (!drag || e.pointerId !== drag.pointerId) return;
    finish(apply);
  };

  const current = step != null && step >= 0 && steps ? steps[step] : null;
  const trackStyle = current && steps
    ? ({ '--mod-tint': stepTint(steps, step ?? 0) } as CSSProperties)
    : undefined;

  return (
    <>
      {step != null && (
        <div
          aria-hidden="true"
          className={`sn-mod-track${current ? ' is-armed' : ''}`}
          style={trackStyle}
        >
          {/* The fill grows by scale, never by width. Ban's rest fills red
              inside it, so it covers exactly the revealed part. */}
          <div className="sn-mod-track-fill">
            {arming && (
              <span className="sn-mod-track-arming" style={{ animationDuration: `${BAN_DWELL_MS}ms` }} />
            )}
          </div>
          {/* A window that opens with the pull over things fixed to the row:
              the ruler notches where each step starts, and Cancel while no
              step is armed. */}
          <div className="sn-mod-track-window">
            <div className="sn-mod-track-window-inner">
              {ruler?.map((x, i) => (
                <span
                  key={x}
                  className={`sn-mod-tick${step != null && i <= step ? ' is-passed' : ''}`}
                  style={{ left: x }}
                />
              ))}
              {!current && (
                <span className="sn-mod-track-cancel">
                  <X size={12} weight="bold" />
                  Cancel
                </span>
              )}
            </div>
          </div>
          {/* The message's shadow where it lifts off the track. */}
          <span className="sn-mod-track-edge" />
          {current && (
            <span key={step} className="sn-mod-track-label">
              {current.action === 'ban' ? (
                <Prohibit size={12} weight="bold" />
              ) : current.action === 'delete' ? (
                <Trash size={12} weight="bold" />
              ) : (
                <Clock size={12} weight="bold" />
              )}
              {current.label}
            </span>
          )}
        </div>
      )}
      <div
        role="slider"
        aria-label="Slide to delete, time out or ban"
        aria-valuemin={0}
        aria-valuemax={steps?.length ?? 0}
        aria-valuenow={step != null && step >= 0 ? step + 1 : 0}
        aria-valuetext={current ? current.label : 'None'}
        data-no-drag="true"
        data-mod-edge="true"
        className={`sn-mod-grip${step != null ? ' is-active' : ''}${current ? ' is-armed' : ''}${hint ? ' is-hinted' : ''}`}
        style={trackStyle}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={(e) => onPointerEnd(e, true)}
        onPointerCancel={(e) => onPointerEnd(e, false)}
        onLostPointerCapture={(e) => onPointerEnd(e, false)}
      >
        <div className="sn-mod-grip-bar" />
      </div>
    </>
  );
}
