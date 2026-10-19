// Swipe right on a message to moderate it: the desktop mod slider with a
// finger, the same on Android as on the iPhone. Rust lays out the ruler and
// says every word (`get_touch_mod_slider`, services/touch_mod_slider.rs); this
// follows the finger, draws the chosen style over the row, buzzes on each step
// and on release sends the armed step through the one mod door.
//
// One move, one direction: the pull only goes right. Short of the dead zone, or
// with the finger strayed `leash` px above or below the message, the chip says
// "Let go to cancel" and a release does nothing. Ban sits at the end of the
// pull and arms only after the finger rests there; until then the slider shows
// (and a release applies) the step before it.
//
// It listens on the list element natively and keeps its own state, so a moving
// finger re-renders this overlay and nothing else: the pane's re-render storms
// are what once dropped chat swipes. The list is held (`holdChatRow`) while the
// finger is down, and a non-passive touchmove stops it scrolling under the pull.
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { holdChatRow, moderateWithToast } from '../../services/moderation';
import { stepAction, stepKeeping, stepTint, type SwipeTarget, type TouchSlider } from './swipeSteps';

export type { SwipeTarget, TouchSlider } from './swipeSteps';
import { hapticCommit, hapticDestructive, hapticStep } from '../ui/haptics';

interface View {
  /** The row, relative to the list element. */
  top: number;
  left: number;
  width: number;
  height: number;
  slide: number;
  step: number | null;
  offRow: boolean;
  /** Ban's rest is running. */
  resting: boolean;
}

interface Props {
  listRef: React.RefObject<HTMLElement | null>;
  slider: TouchSlider | null;
  broadcasterId: string | null;
  /** The chatter behind a row the viewer may swipe, or null. */
  targetFor: (messageId: string) => SwipeTarget | null;
  /** The long-press fan owns the finger once armed. */
  fanArmed: () => boolean;
}

export function SwipeModerator({ listRef, slider, broadcasterId, targetFor, fanArmed }: Props) {
  const [view, setView] = useState<View | null>(null);
  // Everything the native listeners read, kept current without re-binding them.
  const live = useRef({ slider, broadcasterId, targetFor, fanArmed });
  live.current = { slider, broadcasterId, targetFor, fanArmed };

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    let press: { id: number; x: number; y: number; row: HTMLElement; messageId: string } | null = null;
    let engaged = false;
    let step: number | null = null;
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
    const end = (apply: boolean) => {
      const p = press;
      const s = live.current.slider;
      press = null;
      stopRest();
      if (p && engaged) {
        restore(p.row);
        holdChatRow(p.row, false);
        const armedStep = step != null && s ? s.steps[step] : null;
        const target = live.current.targetFor(p.messageId);
        const bid = live.current.broadcasterId;
        if (apply && armedStep && target && bid) {
          if (armedStep.action === 'delete') hapticCommit();
          else hapticDestructive();
          void moderateWithToast({
            provider: 'twitch',
            broadcasterId: bid,
            targetUserId: target.userId,
            targetName: target.name,
            action: stepAction(armedStep, p.messageId),
            useSavedReason: armedStep.action !== 'delete',
          });
        }
      }
      engaged = false;
      step = null;
      setView(null);
    };

    const draw = (slide: number, offRow: boolean) => {
      const s = live.current.slider;
      if (!s || !press || !rowRect || !listRect) return;
      const b = body(press.row);
      const armed = step != null && !offRow;
      if (s.style === 'capsule') press.row.style.transform = `translateX(${slide}px)`;
      if (s.style === 'preview' && b) {
        b.style.opacity = armed ? '0.45' : '';
        b.style.textDecoration = armed ? `line-through 1.5px ${stepTint(s.steps[step!])}` : '';
      }
      setView({
        top: rowRect.top - listRect.top,
        left: rowRect.left - listRect.left,
        width: rowRect.width,
        height: rowRect.height,
        slide,
        step,
        offRow,
        resting: rest != null && !rest.armed,
      });
    };

    const onDown = (e: PointerEvent) => {
      const s = live.current.slider;
      if (!e.isPrimary || !s?.enabled || press) return;
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-message-id]');
      const messageId = row?.getAttribute('data-message-id');
      if (!row || !messageId || !live.current.targetFor(messageId)) return;
      press = { id: e.pointerId, x: e.clientX, y: e.clientY, row, messageId };
    };

    const onMove = (e: PointerEvent) => {
      const s = live.current.slider;
      if (!press || e.pointerId !== press.id || !s) return;
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      if (!engaged) {
        if (live.current.fanArmed()) {
          press = null;
          return;
        }
        // Sideways and rightward claims the finger; anything else is the list's scroll.
        if (Math.abs(dy) > s.engage && Math.abs(dy) >= Math.abs(dx)) {
          press = null;
          return;
        }
        if (dx < s.engage || dx < Math.abs(dy) * 1.5) return;
        engaged = true;
        // The pull may leave the list; its release still comes here.
        try {
          list.setPointerCapture(e.pointerId);
        } catch {
          // The pointer is already gone; the next up or cancel ends the pull.
        }
        rowRect = press.row.getBoundingClientRect();
        listRect = list.getBoundingClientRect();
        holdChatRow(press.row, true);
      }
      const slide = Math.min(Math.max(0, dx), s.end - 0.5);
      const offRow = rowRect != null && (e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash);
      let next = stepKeeping(s, slide, step);
      if (next != null && s.steps[next].action === 'ban') {
        if (!rest || Math.abs(slide - rest.anchor) > s.ban_still) {
          stopRest();
          const anchor = slide;
          rest = {
            anchor,
            armed: false,
            timer: setTimeout(() => {
              if (!rest) return;
              rest.armed = true;
              step = s.steps.length - 1;
              hapticStep();
              draw(anchor, false);
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
      draw(slide, offRow);
    };

    const onUp = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const s = live.current.slider;
      const offRow = rowRect != null && s != null && (e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash);
      end(!offRow);
    };
    const onCancel = () => end(false);
    const onTouchMove = (e: TouchEvent) => {
      if (engaged) e.preventDefault();
    };
    const onHidden = () => document.visibilityState === 'hidden' && end(false);

    list.addEventListener('pointerdown', onDown);
    list.addEventListener('pointermove', onMove);
    list.addEventListener('pointerup', onUp);
    list.addEventListener('pointercancel', onCancel);
    list.addEventListener('touchmove', onTouchMove, { passive: false });
    window.addEventListener('blur', onCancel);
    document.addEventListener('visibilitychange', onHidden);
    return () => {
      end(false);
      list.removeEventListener('pointerdown', onDown);
      list.removeEventListener('pointermove', onMove);
      list.removeEventListener('pointerup', onUp);
      list.removeEventListener('pointercancel', onCancel);
      list.removeEventListener('touchmove', onTouchMove);
      window.removeEventListener('blur', onCancel);
      document.removeEventListener('visibilitychange', onHidden);
    };
  }, [listRef]);

  const host = listRef.current;
  if (!view || !slider || !host) return null;
  return createPortal(<SwipeOverlay view={view} slider={slider} />, host);
}

function SwipeOverlay({ view, slider }: { view: View; slider: TouchSlider }) {
  const step = view.offRow ? null : view.step;
  const tint = step != null ? stepTint(slider.steps[step]) : 'var(--color-text-secondary)';
  const words =
    step == null
      ? slider.cancel_words
      : view.resting
        ? `${slider.steps[step].label}, ${slider.hold_ban_words}`
        : slider.steps[step].chip;
  // Above the finger, or under the row when the row sits at the top of the list.
  const chipAbove = view.top > 44;
  const chipX = Math.min(Math.max(0, view.slide - 60), view.width - 140);
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
            const center = (slider.starts[i] + zoneEnd) / 2;
            const lit = i === step;
            // glass-exempt: the lit step's tint over the rail, a color cue that must stay translucent.
            const pill = lit ? `color-mix(in srgb, ${tint} 22%, transparent)` : undefined;
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
        <div className="absolute inset-y-0 left-0" style={{ width: view.slide, background: `color-mix(in srgb, ${tint} 18%, transparent)` }} />
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
                style={{ left: start + 1, width: Math.max(2, zoneEnd - start - 2), background: stepTint(s), opacity: start <= view.slide ? 1 : 0.32 }}
              />
            );
          })}
        </div>
      )}
      <div
        className="absolute flex h-7 min-w-[120px] items-center justify-center whitespace-nowrap rounded-xl border bg-surface px-3 text-[13px] font-semibold"
        style={{
          left: chipX,
          top: chipAbove ? -34 : view.height + 6,
          borderColor: `color-mix(in srgb, ${tint} 55%, transparent)`,
          color: step == null ? 'var(--color-text-secondary)' : 'var(--color-text-primary)',
        }}
      >
        {words}
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
