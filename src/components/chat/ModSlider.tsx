import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { Clock, Prohibit, Trash } from 'phosphor-react';

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
import { stepTint } from './modStepTint';

/**
 * The slider on a chat row's left edge, for moderators: drag right through
 * the steps to Ban, let go to apply; pull back to the start, leave the row
 * or press Escape to cancel.
 *
 * Presentation only. The steps are Rust's (`get_mod_slider_steps`: Delete, the
 * timeouts the platform will really apply, Ban, mildest first) and the action
 * goes through `moderate_chatter`.
 *
 * One gesture, three looks (`chat_design.mod_slider_style`):
 * - preview: the message stays put; its text is struck through in the step's
 *   color while a chip at the pointer names the step.
 * - rail: a glass rail over the message lays out every step where it sits,
 *   and a lit pill snaps from step to step.
 * - capsule: the message slides aside for a capsule that stretches with the
 *   pull; the step name rolls like an odometer.
 * Whatever the look, the chip says in words when a release will do nothing.
 *
 * The row is driven through attributes and CSS variables written straight
 * onto its element (`data-mod-slide`, `--mod-slide`, `--mod-tint`), so a drag
 * frame never re-renders the row and everything that follows the pointer
 * moves by transform alone: no layout, no repaint. This component renders
 * again only when the step changes. Mounted only on a hovered row, so idle
 * rows carry none of it.
 */

export type SliderStyle = 'preview' | 'rail' | 'capsule';

/** Pull this far before the first step; less is the cancel zone. Wide
 *  enough that the first step's label fits once it is armed. A narrow row
 *  takes a quarter of its width instead. */
const DEAD_ZONE_PX = 72;
/** Room per step by the Length setting, the same at every chat width so a
 *  pull learned once lands on the same step everywhere. A row too narrow for
 *  it squeezes the steps to fit, never below the floor. */
const STEP_PX = { short: 18, medium: 22, long: 28 } as const;
const STEP_FLOOR_PX = 12;
/** Ban sits this many extra step-lengths past the last timeout, so it is
 *  never reached by a pull that only overshot the longest timeout. */
const BAN_GAP_STEPS = 1;
/** Ban's zone is at least this long. The pull stops at its end: the handle
 *  goes no further, so there is no invisible line to overshoot. */
const BAN_ZONE_MIN_PX = 28;
const RETURN_MS = 220;
/** Ban arms only after the pointer rests this long in its zone. Until then
 *  the slider shows the step before it and a release applies exactly that,
 *  so a flick past the end never bans. */
const BAN_DWELL_MS = 350;
/** Moving this far inside Ban's zone starts the rest over. */
const BAN_STILL_PX = 6;
/** A step changes only this far past its edge, so a hand resting on the
 *  line between two steps does not flicker between them. */
const HYSTERESIS_PX = 3;
/** The pointer may stray this far above or below the row; further away a
 *  release cancels. Wide: a long pull to the right drifts up or down, and a
 *  cancel there must take a deliberate move off the message. */
const LEASH_PX = 120;
/** Preview keeps the struck-through look this long after a release, so the
 *  line does not flash back before the platform's own removal arrives. */
const APPLIED_HOLD_MS = 1500;
/** How long "Drag right to act" stays after a press that did not drag. */
const NUDGE_MS = 1500;
/** The rail writes every step's name when a step has this much room; with
 *  less, the steps not under the pointer are dots. */
const RAIL_LABEL_MIN_PX = 26;
/** One line of the capsule's rolling label. Matches .sn-mod-odo in CSS. */
const ODO_LINE_PX = 14;
/** The chip sits this far above the pointer; with less room above it in the
 *  list (under the floating header too), it goes below the pointer. */
const CHIP_ROOM_PX = 34;

/** Pulled, but a release does nothing: in the cancel zone or off the row.
 *  0 and up is a step. */
const CANCEL_ZONE = -1;
const OFF_ROW = -3;

const CANCEL_TINT = 'var(--color-text-secondary)';

interface DragState {
  pointerId: number;
  startX: number;
  row: HTMLElement;
  rect: DOMRect;
  dead: number;
  spacing: number;
  ruler: number[];
  /** The pull stops here: the end of Ban's zone, or of the last step. */
  end: number;
  engaged: boolean;
  step: number;
  /** Running while the pull rests in Ban's zone before Ban arms. */
  banTimer: number | null;
  /** Where that rest started. */
  banAnchor: number;
  detach: () => void;
}

/** Where things sit, fixed for one press. */
interface Geometry {
  /** The press, from the row's left edge: the pointer is at origin + pull. */
  origin: number;
  spacing: number;
  /** Each step's middle, from the row's left edge (rail). */
  centers: number[];
  /** Where each step starts and where the steps end, from the row's left
   *  edge (the preview's track). */
  ticks: number[];
  trackStart: number;
  trackEnd: number;
  /** The chip sits under the pointer instead of over it. */
  below: boolean;
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

/** Step-lengths the steps span: one per step, plus the gap before Ban. */
function stepUnits(steps: ModStep[]): number {
  const endsInBan = steps[steps.length - 1]?.action === 'ban';
  return steps.length + (endsInBan ? BAN_GAP_STEPS : 0);
}

/** Where each step's zone starts, as a pull in px. */
function rulerFor(steps: ModStep[], dead: number, spacing: number): number[] {
  const last = steps.length - 1;
  return steps.map((s, i) => dead + (s.action === 'ban' && i === last ? i + BAN_GAP_STEPS : i) * spacing);
}

/** The step under a pull of `slide` px, or CANCEL_ZONE. The gap before Ban
 *  keeps showing the step before it (or nothing, when Ban is the only
 *  step). */
function stepAt(steps: ModStep[], drag: DragState, slide: number): number {
  if (slide < drag.dead) return CANCEL_ZONE;
  const unit = Math.floor((slide - drag.dead) / drag.spacing);
  const last = steps.length - 1;
  if (steps[last]?.action !== 'ban') return Math.min(last, unit);
  if (unit >= last + BAN_GAP_STEPS) return last;
  return Math.min(unit, last - 1);
}

/** The step to show, keeping the current one until the pull is a few px
 *  past its edge. */
function withHysteresis(drag: DragState, slide: number, next: number): number {
  const cur = drag.step;
  if (cur < 0 || next < 0 || next === cur) return next;
  if (next > cur) return slide >= drag.ruler[next] + HYSTERESIS_PX ? next : cur;
  return slide <= drag.ruler[cur] - HYSTERESIS_PX ? next : cur;
}

/** True when the chip has no clear room above the pointer, so it goes below.
 *  Asks the page what is really drawn where the chip would sit: anything that
 *  is not the chat list itself there (the chat header and whatever it holds,
 *  a pinned message, a pane's border, the window's edge) means no room. The
 *  list's own top and the header inset it reserves are checked too, for
 *  overlays that let the pointer through. */
function chipBelow(row: HTMLElement, pointerX: number, pointerY: number): boolean {
  const top = pointerY - CHIP_ROOM_PX;
  if (top < 0) return true;
  let list: HTMLElement | null = null;
  for (let el = row.parentElement; el; el = el.parentElement) {
    if (getComputedStyle(el).overflowY !== 'visible') {
      list = el;
      break;
    }
  }
  if (list) {
    const inset = Number(row.closest<HTMLElement>('[data-header-inset]')?.dataset.headerInset ?? 0) || 0;
    if (top < list.getBoundingClientRect().top + inset) return true;
  }
  // Along the chip's top edge, over the stretch of the row it starts in
  // (it is kept inside the row, so never left of its edge).
  const rect = row.getBoundingClientRect();
  for (const dx of [8, 48, 96]) {
    const x = Math.min(Math.max(pointerX, rect.left) + dx, rect.right - 4);
    const hit = document.elementFromPoint(x, top + 2);
    if (!hit || (list ? !list.contains(hit) : !row.parentElement?.contains(hit))) return true;
  }
  return false;
}

function actionFor(step: ModStep, messageId: string | undefined): ModAction | null {
  if (step.action === 'delete') return messageId ? { kind: 'delete', messageId } : null;
  if (step.action === 'ban') return { kind: 'ban' };
  return step.seconds != null ? { kind: 'timeout', seconds: step.seconds } : null;
}

function StepIcon({ step }: { step: ModStep }) {
  if (step.action === 'ban') return <Prohibit size={12} weight="bold" />;
  if (step.action === 'delete') return <Trash size={12} weight="bold" />;
  return <Clock size={12} weight="bold" />;
}

/** Icon and name of a step. */
function stepLabel(step: ModStep, suffix?: string): ReactNode {
  return (
    <>
      <StepIcon step={step} />
      {suffix ? `${step.label}${suffix}` : step.label}
    </>
  );
}

/** A release that does nothing, in words. Words only: an X in a pill reads
 *  as a button, and nothing here can be pressed. */
function cancelLabel(): ReactNode {
  return 'Let go to cancel';
}

/** A step's mark on the rail: Delete and Ban by icon (Ban named too, it has
 *  the room), timeouts by their length, or a dot when the rail is tight. */
function railMark(step: ModStep, dots: boolean): ReactNode {
  if (step.action === 'delete') return <Trash size={12} weight="bold" />;
  if (step.action === 'ban') {
    return (
      <>
        <Prohibit size={12} weight="bold" />
        {step.label}
      </>
    );
  }
  return dots ? <i className="sn-mod-rail-dot" /> : step.label;
}

export function ModSlider({ provider, target, messageId, onApply, hint = false }: ModSliderProps) {
  const allSteps = useModSliderSteps(provider, useSliderConfig());
  const length = useAppStore((s) => s.settings.chat_design?.mod_slider_length) ?? 'medium';
  const storedStyle = useAppStore((s) => s.settings.chat_design?.mod_slider_style);
  const look: SliderStyle = storedStyle === 'rail' || storedStyle === 'capsule' ? storedStyle : 'preview';
  const stepPx = STEP_PX[length] ?? STEP_PX.medium;
  const steps = messageId ? allSteps : allSteps?.filter((s) => s.action !== 'delete') ?? null;
  // null = idle; CANCEL_ZONE or OFF_ROW = pulled, a release does nothing;
  // 0+ = the step.
  const [step, setStep] = useState<number | null>(null);
  // True while the pull rests in Ban's zone and Ban is not armed yet.
  const [arming, setArming] = useState(false);
  // Counts each start of that rest, so its fill starts over with it.
  const [armSeq, setArmSeq] = useState(0);
  const [geom, setGeom] = useState<Geometry | null>(null);
  // "Drag right to act", after a press that did not drag.
  const [nudge, setNudge] = useState(false);
  // The look of the drag in progress; a settings change mid-drag waits.
  const [dragLook, setDragLook] = useState<SliderStyle>(look);
  const dragRef = useRef<DragState | null>(null);
  // The row the last drag drove; outlives the drag for the return and hold.
  const rowRef = useRef<HTMLElement | null>(null);
  const chipRef = useRef<HTMLSpanElement | null>(null);
  const returnTimerRef = useRef<number | null>(null);
  const appliedTimerRef = useRef<number | null>(null);
  const nudgeTimerRef = useRef<number | null>(null);

  /** Everything the drag wrote on the row, gone. */
  const settle = (row: HTMLElement, keepApplied = false) => {
    delete row.dataset.modSlide;
    delete row.dataset.modPhase;
    delete row.dataset.modArmed;
    const vars = ['--mod-slide', '--mod-slide-n', '--mod-origin', '--mod-row-w', '--mod-chip-half', '--mod-press-y'];
    if (keepApplied) {
      // The held strike only needs the tint.
    } else {
      delete row.dataset.modApplied;
      vars.push('--mod-tint');
    }
    if (!row.dataset.modHint) for (const v of vars) row.style.removeProperty(v);
  };

  const clearApplied = (row: HTMLElement) => {
    if (appliedTimerRef.current != null) {
      window.clearTimeout(appliedTimerRef.current);
      appliedTimerRef.current = null;
    }
    delete row.dataset.modApplied;
    if (!row.dataset.modSlide) row.style.removeProperty('--mod-tint');
  };

  const clearNudge = (row: HTMLElement | null) => {
    if (nudgeTimerRef.current != null) {
      window.clearTimeout(nudgeTimerRef.current);
      nudgeTimerRef.current = null;
    }
    if (row?.dataset.modHint) {
      delete row.dataset.modHint;
      settle(row);
    }
    setNudge(false);
  };

  /** The step's color and armed state, onto the row. */
  const paint = (row: HTMLElement, index: number) => {
    if (!steps) return;
    row.style.setProperty('--mod-tint', index >= 0 ? stepTint(steps, index) : CANCEL_TINT);
    if (index >= 0) row.dataset.modArmed = steps[index].action;
    else delete row.dataset.modArmed;
  };

  const finish = (apply: boolean) => {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    if (drag.banTimer != null) window.clearTimeout(drag.banTimer);
    drag.detach();
    setArming(false);
    const { row, engaged, step: chosen } = drag;
    holdChatRow(row, false);
    if (!engaged) {
      setStep(null);
      // A press that never dragged right: say what the tab is for.
      if (apply) {
        row.dataset.modHint = 'true';
        setNudge(true);
        nudgeTimerRef.current = window.setTimeout(() => {
          nudgeTimerRef.current = null;
          clearNudge(row);
          setGeom(null);
        }, NUDGE_MS);
      } else {
        settle(row);
        setGeom(null);
      }
      return;
    }
    const picked = apply && chosen >= 0 ? steps?.[chosen] : undefined;
    // Preview holds its struck-through look after a release, so letting go
    // shows nothing that was not already there.
    const hold = !!picked && dragLook === 'preview';
    if (hold) {
      row.dataset.modApplied = picked.action;
      appliedTimerRef.current = window.setTimeout(() => {
        appliedTimerRef.current = null;
        clearApplied(row);
      }, APPLIED_HOLD_MS);
    } else if (!picked) {
      // A release that applied keeps its step's color while it springs back.
      delete row.dataset.modArmed;
    }
    // Spring back, then clear.
    row.dataset.modPhase = 'return';
    row.style.setProperty('--mod-slide', '0px');
    row.style.setProperty('--mod-slide-n', '0');
    returnTimerRef.current = window.setTimeout(() => {
      returnTimerRef.current = null;
      settle(row, hold);
      setStep(null);
      setGeom(null);
    }, RETURN_MS);

    if (!picked) return;
    if (onApply) {
      onApply(picked);
      return;
    }
    const action = actionFor(picked, messageId);
    if (!action) return;
    // A refused action or an Undo takes the held strike off at once.
    void moderateWithToast(
      { ...target, action, useSavedReason: action.kind !== 'delete' },
      { onUndo: () => clearApplied(row) },
    ).then((outcome) => {
      if (!outcome) clearApplied(row);
    });
  };
  // The listeners added at the press call the newest finish.
  const finishRef = useRef(finish);
  useLayoutEffect(() => {
    finishRef.current = finish;
  });

  // A row that leaves the list mid-drag cancels; nothing is ever applied on
  // unmount.
  useEffect(
    () => () => {
      if (returnTimerRef.current != null) window.clearTimeout(returnTimerRef.current);
      if (appliedTimerRef.current != null) window.clearTimeout(appliedTimerRef.current);
      if (nudgeTimerRef.current != null) window.clearTimeout(nudgeTimerRef.current);
      const drag = dragRef.current;
      if (drag) {
        dragRef.current = null;
        if (drag.banTimer != null) window.clearTimeout(drag.banTimer);
        drag.detach();
      }
      const row = rowRef.current;
      if (row) {
        delete row.dataset.modHint;
        settle(row);
      }
    },
    [],
  );

  // The chip is kept inside the row by half its width, which changes with
  // what it says.
  useLayoutEffect(() => {
    const row = rowRef.current;
    const chip = chipRef.current;
    if (!row || !chip) return;
    row.style.setProperty('--mod-chip-half', `${chip.offsetWidth / 2}px`);
  }, [step, arming, dragLook, nudge]);

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
    }
    clearNudge(rowRef.current);
    settle(row);
    clearApplied(row);
    rowRef.current = row;
    capturePointer(e.currentTarget, e.pointerId);
    // Hold the list from the press, not the first movement: in a fast chat
    // the row would otherwise move before the drag even starts.
    holdChatRow(row, true);
    const rect = row.getBoundingClientRect();
    const dead = Math.min(DEAD_ZONE_PX, Math.round(rect.width * 0.25));
    const endsInBan = steps[steps.length - 1]?.action === 'ban';
    // Room for every step, Ban's longer zone and a little to spare.
    const fit = (rect.width - dead - 24) / (stepUnits(steps) + (endsInBan ? 0.5 : 0));
    const spacing = Math.max(STEP_FLOOR_PX, Math.min(stepPx, fit));
    const ruler = rulerFor(steps, dead, spacing);
    const banStart = ruler[ruler.length - 1];
    const lastZone = endsInBan ? Math.max(1.5 * spacing, BAN_ZONE_MIN_PX) : spacing;
    const end = banStart + lastZone;
    const origin = e.clientX - rect.left;
    // Alt+Tab or a hidden window mid-drag cancels.
    const onKey = (ke: KeyboardEvent) => {
      if (ke.key !== 'Escape') return;
      ke.preventDefault();
      ke.stopPropagation();
      finishRef.current(false);
    };
    const onLeave = () => finishRef.current(false);
    const onVisibility = () => {
      if (document.hidden) finishRef.current(false);
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('blur', onLeave);
    document.addEventListener('visibilitychange', onVisibility);
    const detach = () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('blur', onLeave);
      document.removeEventListener('visibilitychange', onVisibility);
    };
    dragRef.current = {
      pointerId: e.pointerId,
      startX: e.clientX,
      row,
      rect,
      dead,
      spacing,
      ruler,
      end,
      engaged: false,
      step: CANCEL_ZONE,
      banTimer: null,
      banAnchor: 0,
      detach,
    };
    setGeom({
      origin,
      spacing,
      centers: ruler.map((x, i) => origin + x + (i === ruler.length - 1 ? lastZone : spacing) / 2),
      ticks: ruler.map((x) => origin + x),
      trackStart: origin + dead,
      trackEnd: origin + end,
      below: chipBelow(row, e.clientX, e.clientY),
    });
    row.style.setProperty('--mod-origin', `${origin}px`);
    row.style.setProperty('--mod-row-w', `${rect.width}px`);
    row.style.setProperty('--mod-press-y', `${e.clientY - rect.top}px`);
    row.style.setProperty('--mod-slide', '0px');
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || e.pointerId !== drag.pointerId || !steps) return;
    // The button came up somewhere this element never heard about.
    if (e.buttons === 0) {
      finish(false);
      return;
    }
    const dx = e.clientX - drag.startX;
    const { row } = drag;
    if (!drag.engaged) {
      if (dx < 4) return;
      drag.engaged = true;
      setDragLook(look);
      row.dataset.modSlide = look;
      row.dataset.modPhase = 'drag';
      paint(row, CANCEL_ZONE);
      // The slider (and its Cancel) shows from the first pixel, not only
      // once a step arms.
      setStep(CANCEL_ZONE);
    }
    // The handle stops at the end; a harder pull stays on the last step.
    const slide = Math.min(Math.max(0, dx), drag.end - 0.5);
    row.style.setProperty('--mod-slide', `${slide}px`);
    row.style.setProperty('--mod-slide-n', String(slide));
    const offRow = e.clientY < drag.rect.top - LEASH_PX || e.clientY > drag.rect.bottom + LEASH_PX;
    let next = offRow ? OFF_ROW : withHysteresis(drag, slide, stepAt(steps, drag, slide));
    const ban = steps.length - 1;
    const inBanZone = next === ban && steps[ban].action === 'ban';
    // Ban arms after the pointer rests in its zone; moving starts it over.
    if (inBanZone && drag.step !== ban) {
      if (drag.banTimer != null && Math.abs(slide - drag.banAnchor) > BAN_STILL_PX) {
        window.clearTimeout(drag.banTimer);
        drag.banTimer = null;
      }
      if (drag.banTimer == null) {
        drag.banAnchor = slide;
        drag.banTimer = window.setTimeout(() => {
          if (dragRef.current !== drag) return;
          drag.banTimer = null;
          drag.step = ban;
          paint(row, ban);
          setArming(false);
          setStep(ban);
        }, BAN_DWELL_MS);
        setArming(true);
        setArmSeq((n) => n + 1);
      }
      // Not armed yet: the step before Ban stays.
      next = ban - 1;
    } else if (!inBanZone && drag.banTimer != null) {
      window.clearTimeout(drag.banTimer);
      drag.banTimer = null;
      setArming(false);
    }
    if (next !== drag.step) {
      drag.step = next;
      paint(row, next);
      setStep(next);
    }
  };

  const onPointerEnd = (e: ReactPointerEvent<HTMLDivElement>, apply: boolean) => {
    const drag = dragRef.current;
    if (!drag || e.pointerId !== drag.pointerId) return;
    finish(apply);
  };

  const current = step != null && step >= 0 && steps ? steps[step] : null;
  const layer = step != null && steps && geom ? { steps, geom } : null;
  const dwell = arming ? (
    <span key={armSeq} className="sn-mod-dwell" style={{ animationDuration: `${BAN_DWELL_MS}ms` }} />
  ) : null;
  const railDots = !!geom && geom.spacing < RAIL_LABEL_MIN_PX;
  // The preview track's segments, each step from its start to the next one's
  // (the last timeout's runs up to Ban), a pixel apart.
  const trackSegments =
    geom && steps
      ? steps.map((s, i) => {
          const from = geom.ticks[i];
          const to = i + 1 < geom.ticks.length ? geom.ticks[i + 1] : geom.trackEnd;
          return (
            <span
              key={`${s.action}-${s.seconds ?? ''}`}
              className="sn-mod-track-seg"
              style={{ left: from - geom.trackStart, width: Math.max(1, to - from - 1), background: stepTint(steps, i) }}
            />
          );
        })
      : null;
  const lastIndex = (steps?.length ?? 0) - 1;
  // The chip: always in Preview; in the rail and the capsule only when a
  // release would do nothing, while Ban waits, or after a press that did not
  // drag, so those words are in one place for every look.
  const chipText: ReactNode = nudge
    ? 'Drag right to act'
    : step == null
      ? null
      : step < 0
        ? cancelLabel()
        : arming && current
          ? stepLabel(current, ', hold for Ban')
          : dragLook === 'preview' && current
            ? stepLabel(current)
            : null;

  return (
    <>
      {layer && dragLook === 'preview' && (
        <>
          {/* A wash of the step's color from the edge to the pointer, behind
              the text. */}
          <span aria-hidden="true" className="sn-mod-layer sn-mod-wash" />
          {/* The track along the bottom edge: one segment per step in that
              step's color, faint ahead of the pointer and lit behind it, and
              a cap where the pull stops. */}
          <span
            aria-hidden="true"
            className="sn-mod-layer sn-mod-track"
            style={
              {
                left: layer.geom.trackStart,
                width: layer.geom.trackEnd - layer.geom.trackStart,
                '--mod-track-from': `${layer.geom.trackStart - layer.geom.origin}px`,
                '--mod-track-len': `${layer.geom.trackEnd - layer.geom.trackStart}px`,
              } as CSSProperties
            }
          >
            <span className="sn-mod-track-base">{trackSegments}</span>
            <span className="sn-mod-track-lit">
              <span className="sn-mod-track-lit-in">{trackSegments}</span>
            </span>
          </span>
        </>
      )}
      {chipText != null && geom && (
        <span aria-hidden="true" className={`sn-mod-layer sn-mod-chip${geom.below ? ' is-below' : ''}`}>
          <span ref={chipRef} className="sn-mod-chip-in">
            <span key={`${step}-${arming}-${nudge}`} className="sn-mod-chip-label">
              {chipText}
            </span>
            {dragLook === 'preview' && dwell}
          </span>
        </span>
      )}
      {layer && dragLook === 'rail' && (
        <div aria-hidden="true" className={`sn-mod-layer sn-mod-rail${railDots ? ' is-dots' : ''}`}>
          {layer.steps.map((s, i) => (
            <span
              key={`${s.action}-${s.seconds ?? ''}`}
              className={`sn-mod-rail-mark${i === step ? ' is-on' : ''}${step != null && i < step ? ' is-passed' : ''}`}
              style={{ transform: `translateX(${layer.geom.centers[i]}px)` }}
            >
              {railMark(s, railDots)}
              {i === lastIndex && dwell}
            </span>
          ))}
          {/* A hairline under the pointer, so the rail feels held; the pill
              covers it. */}
          <span className="sn-mod-rail-needle" />
          {/* The lit pill: centered on the armed step, it snaps there. None
              while a release does nothing (the chip says so). */}
          {current && step != null && (
            <span className="sn-mod-rail-thumb" style={{ transform: `translateX(${layer.geom.centers[step]}px)` }}>
              <span className="sn-mod-rail-thumb-in">
                {current.action === 'delete' ? <Trash size={12} weight="bold" /> : stepLabel(current)}
              </span>
            </span>
          )}
        </div>
      )}
      {layer && dragLook === 'capsule' && (
        <span aria-hidden="true" className="sn-mod-layer sn-mod-cap-clip">
          <span className="sn-mod-cap">
            {arming && <span key={armSeq} className="sn-mod-cap-dwell" style={{ animationDuration: `${BAN_DWELL_MS}ms` }} />}
            <span className="sn-mod-odo">
              <span className="sn-mod-odo-stack" style={{ transform: `translateY(${-((step ?? -1) < 0 ? 0 : (step ?? 0) + 1) * ODO_LINE_PX}px)` }}>
                {/* Cancel is a blank line (the chip says it in words); Ban
                    names who it bans, since the message has slid aside. */}
                {[null, ...layer.steps].map((s, i) => (
                  <span key={i} className="sn-mod-odo-line">
                    {!s ? null : s.action === 'ban' && target.targetName ? (
                      stepLabel(s, ` ${target.targetName}`)
                    ) : (
                      stepLabel(s)
                    )}
                  </span>
                ))}
              </span>
            </span>
          </span>
        </span>
      )}
      <div
        // A pointer-only control: the keyboard path is J/K and the action
        // keys (ChatWidget).
        aria-hidden="true"
        data-no-drag="true"
        data-mod-edge="true"
        className={`sn-mod-layer sn-mod-grip${step != null ? ' is-active' : ''}${hint ? ' is-hinted' : ''}`}
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
