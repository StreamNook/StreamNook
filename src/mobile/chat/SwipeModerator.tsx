// Swipe right on a message to moderate it: the desktop mod slider with a
// finger, the same on Android as on the iPhone. Rust lays the slider out for
// each press, from where the thumb landed, and says every word with the
// chatter's name in it (`get_touch_mod_slider`, services/touch_mod_slider.rs);
// this follows the finger, draws the chosen style over the row, buzzes on each
// step and on release sends the armed step through the one mod door.
//
// One move, one direction: the pull only goes right. Short of the cancel zone,
// or with the finger strayed `leash` px above or below the message, the chip
// says "Let go to cancel" and a release does nothing. Ban sits at the end of
// the pull and arms only after the finger rests there; a release before it
// arms cancels, so a flick past the end does nothing at all.
//
// The thumb landing on a row the viewer moderates holds the list at once
// (`holdChatRow`) and captures the chatter, so in a fast chat the row pressed
// is the row acted on. It listens on the list element natively and keeps its
// own state, so a moving finger re-renders this overlay and nothing else: the
// pane's re-render storms are what once dropped chat swipes. A non-passive
// touchmove stops the list scrolling under the pull.
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { holdChatRow, moderateWithToast } from '../../services/moderation';
import { hapticCommit, hapticDestructive, hapticStep } from '../ui/haptics';
import { stepAction, stepKeeping, stepTint, type SwipeTarget, type TouchSlider } from './swipeSteps';

export type { SwipeTarget, TouchSlider } from './swipeSteps';

interface View {
  /** The row, relative to the list element. */
  top: number;
  left: number;
  width: number;
  height: number;
  slide: number;
  step: number | null;
  offRow: boolean;
  /** Ban's rest is running: a release now cancels. */
  resting: boolean;
}

interface Props {
  listRef: React.RefObject<HTMLElement | null>;
  /** Swiping is on here (moderator tools on, not the read-only overlay). */
  enabled: boolean;
  broadcasterId: string | null;
  /** The chatter behind a row the viewer may swipe, or null. */
  targetFor: (messageId: string) => SwipeTarget | null;
  /** The core's slider for a press `startX` px into a row `width` px wide, on `name`'s message. */
  sliderFor: (width: number, startX: number, name: string) => Promise<TouchSlider | null>;
  /** The long-press fan owns the finger once armed. */
  fanArmed: () => boolean;
  /** A swipe ran (the one-time hint goes). */
  onDone: () => void;
}

export function SwipeModerator({ listRef, enabled, broadcasterId, targetFor, sliderFor, fanArmed, onDone }: Props) {
  const [view, setView] = useState<View | null>(null);
  const [shown, setShown] = useState<TouchSlider | null>(null);
  // Everything the native listeners read, kept current without re-binding them.
  const live = useRef({ enabled, broadcasterId, targetFor, sliderFor, fanArmed, onDone });
  live.current = { enabled, broadcasterId, targetFor, sliderFor, fanArmed, onDone };

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    let press: {
      id: number;
      x: number;
      y: number;
      row: HTMLElement;
      messageId: string;
      target: SwipeTarget;
      slider: TouchSlider | null;
    } | null = null;
    let engaged = false;
    let step: number | null = null;
    let offRow = false;
    let rest: { timer: ReturnType<typeof setTimeout>; anchor: number; armed: boolean } | null = null;
    let rowRect: DOMRect | null = null;
    let listRect: DOMRect | null = null;

    const body = (row: HTMLElement) => row.querySelector<HTMLElement>('[data-mod-body]');
    const restore = (row: HTMLElement) => {
      row.style.transform = '';
      row.style.transition = 'transform 220ms ease-out';
      const b = body(row);
      if (b) {
        b.style.opacity = '';
        b.style.textDecoration = '';
      }
      setTimeout(() => (row.style.transition = ''), 240);
    };
    const stopRest = () => {
      if (rest) clearTimeout(rest.timer);
      rest = null;
    };
    const resting = () => rest != null && !rest.armed;
    /** The press ends: the list is let go, and with `apply` an armed step runs. */
    const end = (apply: boolean) => {
      const p = press;
      press = null;
      const s = p?.slider ?? null;
      const armedStep = apply && engaged && !offRow && !resting() && step != null && s ? s.steps[step] : null;
      stopRest();
      if (p) {
        if (engaged) restore(p.row);
        holdChatRow(p.row, false);
        const bid = live.current.broadcasterId;
        if (armedStep && bid) {
          if (armedStep.action === 'delete') hapticCommit();
          else hapticDestructive();
          void moderateWithToast({
            provider: 'twitch',
            broadcasterId: bid,
            targetUserId: p.target.userId,
            targetName: p.target.name,
            action: stepAction(armedStep, p.messageId),
            useSavedReason: armedStep.action !== 'delete',
          });
          live.current.onDone();
        }
      }
      engaged = false;
      step = null;
      offRow = false;
      setView(null);
    };

    const draw = (slide: number) => {
      const s = press?.slider;
      if (!s || !press || !rowRect || !listRect) return;
      const b = body(press.row);
      const lit = step != null && !offRow && !resting();
      if (s.style === 'capsule') press.row.style.transform = `translateX(${slide}px)`;
      if (s.style === 'preview' && b) {
        b.style.opacity = lit ? '0.45' : '';
        b.style.textDecoration = lit ? `line-through 1.5px ${stepTint(s.steps[step!])}` : '';
      }
      setShown(s);
      setView({
        top: rowRect.top - listRect.top,
        left: rowRect.left - listRect.left,
        width: rowRect.width,
        height: rowRect.height,
        slide,
        step,
        offRow,
        resting: resting(),
      });
    };

    const onDown = (e: PointerEvent) => {
      if (!e.isPrimary || !live.current.enabled || press) return;
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-message-id]');
      const messageId = row?.getAttribute('data-message-id');
      const target = messageId ? live.current.targetFor(messageId) : null;
      if (!row || !messageId || !target) return;
      // Hold the list now: the row under the thumb is the row a swipe acts on.
      holdChatRow(row, true);
      const r = row.getBoundingClientRect();
      const current = { id: e.pointerId, x: e.clientX, y: e.clientY, row, messageId, target, slider: null as TouchSlider | null };
      press = current;
      void live.current.sliderFor(r.width, e.clientX - r.left, target.name).then((s) => {
        if (press === current) current.slider = s;
      });
    };

    const onMove = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const s = press.slider;
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      if (!engaged) {
        const engage = s?.engage ?? 10;
        // A vertical move is the list's scroll, and the fan owns a hold that armed.
        if (live.current.fanArmed() || (Math.abs(dy) > engage && Math.abs(dy) >= Math.abs(dx))) {
          end(false);
          return;
        }
        // Sideways and rightward claims the finger, once the core has laid the slider out.
        if (!s?.enabled || dx < engage || dx < Math.abs(dy) * 1.5) return;
        engaged = true;
        // The pull may leave the list; its release still comes here.
        try {
          list.setPointerCapture(e.pointerId);
        } catch {
          // The pointer is already gone; the next up or cancel ends the pull.
        }
        rowRect = press.row.getBoundingClientRect();
        listRect = list.getBoundingClientRect();
      }
      if (!s) return;
      const slide = Math.min(Math.max(0, dx), s.end - 0.5);
      offRow = rowRect != null && (e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash);
      let next = stepKeeping(s, slide, step);
      if (next != null && s.steps[next].action === 'ban') {
        if (!rest || Math.abs(slide - rest.anchor) > s.ban_still) {
          stopRest();
          const anchor = slide;
          rest = {
            anchor,
            armed: false,
            timer: setTimeout(() => {
              if (!rest || !press) return;
              rest.armed = true;
              step = s.steps.length - 1;
              if (!offRow) hapticCommit();
              draw(anchor);
            }, s.ban_dwell_ms),
          };
        }
        if (!rest.armed) next = next > 0 ? next - 1 : null;
      } else {
        stopRest();
      }
      if (next !== step) {
        step = next;
        if (next != null && !offRow) hapticStep();
      }
      draw(slide);
    };

    const onUp = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const s = press.slider;
      if (s && rowRect) offRow = e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash;
      end(true);
    };
    const onCancel = (e: PointerEvent) => {
      if (press && e.pointerId !== press.id) return;
      end(false);
    };
    const onTouchMove = (e: TouchEvent) => {
      if (engaged) e.preventDefault();
    };
    const onHidden = () => document.visibilityState === 'hidden' && end(false);
    const onBlur = () => end(false);

    list.addEventListener('pointerdown', onDown);
    list.addEventListener('pointermove', onMove);
    list.addEventListener('pointerup', onUp);
    list.addEventListener('pointercancel', onCancel);
    list.addEventListener('touchmove', onTouchMove, { passive: false });
    window.addEventListener('blur', onBlur);
    document.addEventListener('visibilitychange', onHidden);
    return () => {
      end(false);
      list.removeEventListener('pointerdown', onDown);
      list.removeEventListener('pointermove', onMove);
      list.removeEventListener('pointerup', onUp);
      list.removeEventListener('pointercancel', onCancel);
      list.removeEventListener('touchmove', onTouchMove);
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onHidden);
    };
  }, [listRef]);

  const host = listRef.current;
  if (!view || !shown || !host) return null;
  return createPortal(<SwipeOverlay view={view} slider={shown} />, host);
}

/** The list fades its top edge; the chip stays below that. */
const LIST_TOP_CLEAR = 30;
const CHIP_RISE = 34;

function SwipeOverlay({ view, slider }: { view: View; slider: TouchSlider }) {
  const chipRef = useRef<HTMLDivElement>(null);
  const [chipWidth, setChipWidth] = useState(140);
  const step = view.offRow ? null : view.step;
  const lit = step != null && !view.resting;
  const tint = step != null ? stepTint(slider.steps[step]) : 'var(--color-text-secondary)';
  const words = step == null ? slider.cancel_words : view.resting ? slider.hold_ban_words : slider.steps[step].chip;
  // Measured after each change of words, so the chip centers on the finger.
  useEffect(() => {
    if (chipRef.current) setChipWidth(chipRef.current.offsetWidth);
  }, [words]);
  // The ruler starts where the thumb landed.
  const at = (x: number) => slider.start + x;
  const finger = at(view.slide);
  const chipX = Math.min(Math.max(4, finger - chipWidth / 2), view.width - chipWidth - 4);
  // Above the row, kept clear of the list's faded top: near the top it comes
  // down over the row rather than under the finger, where the thumb covers it.
  const chipTop = Math.max(-CHIP_RISE, LIST_TOP_CLEAR - view.top);
  return (
    <div
      className="pointer-events-none absolute z-30"
      style={{ top: view.top, left: view.left, width: view.width, height: view.height }}
      aria-live="polite"
    >
      {slider.style === 'rail' ? (
        <div className="absolute inset-x-1 inset-y-0.5 rounded-[10px] border border-white/[0.08] bg-surface">
          {slider.steps.map((s, i) => {
            const zoneEnd = i + 1 < slider.starts.length ? slider.starts[i + 1] : slider.end;
            const center = at((slider.starts[i] + zoneEnd) / 2) - 4;
            const on = lit && i === step;
            // glass-exempt: the lit step's tint over the rail, a color cue that must stay translucent.
            const pill = on ? `color-mix(in srgb, ${tint} 22%, transparent)` : undefined;
            return (
              <span
                key={`${s.action}-${s.seconds ?? i}`}
                className="absolute top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-[9px] px-2 py-0.5 text-[12px] font-semibold"
                style={{ left: center, color: stepTint(s), background: pill }}
              >
                {s.label}
              </span>
            );
          })}
        </div>
      ) : (
        // glass-exempt: the pull's wash over the message, which must keep the message readable through it.
        <div className="absolute inset-y-0" style={{ left: at(0), width: view.slide, background: `color-mix(in srgb, ${tint} 18%, transparent)` }} />
      )}
      {slider.style === 'preview' && (
        <div className="absolute inset-x-0 bottom-0 h-[3px]">
          {slider.steps.map((s, i) => {
            const start = slider.starts[i];
            const zoneEnd = i + 1 < slider.starts.length ? slider.starts[i + 1] : slider.end;
            return (
              <span
                key={`t-${i}`}
                className="absolute top-0 h-[3px] rounded-full"
                style={{ left: at(start) + 1, width: Math.max(2, zoneEnd - start - 2), background: stepTint(s), opacity: start <= view.slide ? 1 : 0.32 }}
              />
            );
          })}
        </div>
      )}
      <div
        ref={chipRef}
        className="absolute flex h-7 items-center justify-center whitespace-nowrap rounded-xl border bg-surface px-3 text-[13px] font-semibold"
        style={{
          left: chipX,
          top: chipTop,
          maxWidth: view.width - 8,
          borderColor: view.resting ? 'color-mix(in srgb, var(--color-error) 55%, transparent)' : `color-mix(in srgb, ${tint} 55%, transparent)`,
          color: lit ? 'var(--color-text-primary)' : 'var(--color-text-secondary)',
        }}
      >
        <span className="truncate">{words}</span>
        {view.resting && (
          <span
            className="absolute bottom-[2px] left-1.5 h-[2px] rounded-full bg-error"
            style={{ width: 'calc(100% - 12px)', transformOrigin: 'left', animation: `sn-swipe-dwell ${slider.ban_dwell_ms}ms linear forwards` }}
          />
        )}
      </div>
    </div>
  );
}
