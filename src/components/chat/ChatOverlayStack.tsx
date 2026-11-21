import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { usePerformanceQuiet } from '../../utils/lowPower';
import type { StackCardControl, StackKind, StackMore } from './chatTopStack';
import { stackBottomOf } from './stackSpacer';

/**
 * Holds the cards at the top of chat (the hype train, the poll and prediction,
 * the channel's pinned message, the mention context card, the pinned mentions)
 * in one column that floats over the chat list on every surface and never
 * shrinks it (SN-284). Rust lays the column out (chatTopStack.ts): the cards'
 * order, the one that is open, the "N more cards" strip and `maxHeight`, past
 * which the open card's body scrolls inside the column.
 *
 * The column takes taps only on its cards and controls: the gaps and the
 * chat's own rows beside and under it stay the chat's. The host reads the
 * column's drawn bottom (`onBottom`) and gives the list a spacer that tall.
 *
 * Empty renders to zero height, so it never sits over chat when nothing is live.
 */
interface ChatOverlayStackProps {
  /** Where the column starts, in the host's pixels from the top of the chat
   *  column: just under the lowest piece of chat chrome (the header, the
   *  combined-chat bar), measured by the host so a taller header can never land
   *  a card on top of it. */
  top?: number;
  /** The most height the cards may take, 0 for no cap yet. */
  maxHeight?: number;
  /** Receives the column, so the host can place what follows it. */
  stackRef?: (el: HTMLDivElement | null) => void;
  /** Hide the cards (they stay mounted) while another view replaces chat. */
  hidden?: boolean;
  /** The "N more cards" strip: what Rust folded away, drawn last. */
  more?: StackMore | null;
  /** The phone's glaze for the strip. */
  phone?: boolean;
  /** Where the column ends, in the host's pixels (0 when it draws nothing),
   *  with a small gap below it. */
  onBottom?: (bottom: number) => void;
  className?: string;
  children: ReactNode;
}

/** Whether the stack's own 180 ms moves are off: Motion Off and Performance mode. */
export function useStackMotionOff(): boolean {
  const motion = useAppStore((s) => s.settings?.motion_mode);
  const quiet = usePerformanceQuiet();
  return motion === 'off' || quiet;
}

/** Measures a column's height and tells the host where it ends. */
export function useStackBottom(top: number, onBottom?: (bottom: number) => void) {
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  const [height, setHeight] = useState(0);
  useEffect(() => {
    if (!el) return;
    // Only what is drawn counts: cards Rust folded stay mounted, hidden, and the
    // column's own padding is not a stack.
    const read = () => {
      const drawn = Array.from(el.children).some((c) => c.getBoundingClientRect().height > 0);
      setHeight(drawn ? Math.round(el.getBoundingClientRect().height) : 0);
    };
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  const bottom = stackBottomOf(top, height);
  const reported = useRef(-1);
  useEffect(() => {
    if (reported.current === bottom) return;
    reported.current = bottom;
    onBottom?.(bottom);
  }, [bottom, onBottom]);
  return { setEl, height, bottom };
}

export function ChatOverlayStack({
  top,
  maxHeight = 0,
  stackRef,
  hidden = false,
  more = null,
  phone = false,
  onBottom,
  className = '',
  children,
}: ChatOverlayStackProps) {
  const floating = top !== undefined;
  const motionOff = useStackMotionOff();
  const { setEl } = useStackBottom(top ?? 0, onBottom);
  const ref = useCallback(
    (el: HTMLDivElement | null) => {
      setEl(el);
      stackRef?.(el);
    },
    [setEl, stackRef],
  );
  return (
    <div
      ref={ref}
      data-chat-top-stack=""
      className={`${
        floating ? `absolute left-2 right-2 z-40 ${motionOff ? '' : 'transition-[top] duration-[180ms] ease-in-out'}` : 'relative shrink-0'
      } pointer-events-none [:where(&>*)]:pointer-events-auto flex-col gap-1 ${hidden ? 'hidden' : 'flex'} ${className}`}
      style={{ top, maxHeight: maxHeight > 0 ? maxHeight : undefined }}
    >
      {children}
      {more && more.count > 0 && (
        <div
          data-stack-more=""
          // Last in the column whatever order the cards were given.
          style={{ order: 99, minHeight: more.height }}
          className={`pointer-events-none flex shrink-0 items-center justify-center rounded-xl px-3 text-[11.5px] font-semibold leading-4 tabular-nums text-textSecondary ${
            phone ? 'chrome-glaze !rounded-2xl' : 'glass-flyout'
          }`}
        >
          {more.count} more {more.count === 1 ? 'card' : 'cards'}
        </div>
      )}
    </div>
  );
}

/** How a card's drawn height is told to the stack's cache: its form. */
export function measureFormOf(form: StackCardControl['form']): 'strip' | 'open' {
  return form === 'open' || form === 'full' ? 'open' : 'strip';
}

/**
 * One card's place in the column: its order, its measured height per form
 * (the stack keeps the last of each and sends it to Rust), and its folded
 * state. A card Rust folded away, or has no slot for, stays mounted but is not
 * drawn, so a hype train still expires and levels up while folded.
 */
export function StackCard({
  kind,
  control,
  measure,
  scrollOpen = false,
  children,
  className = '',
}: {
  kind: StackKind;
  control: StackCardControl;
  measure?: (kind: StackKind, form: 'strip' | 'open' | 'line', height: number) => void;
  /** An open card is held to the height Rust gave it and scrolls inside it (its
   *  natural height is what the stack's cache keeps, not the capped one). */
  scrollOpen?: boolean;
  children: ReactNode;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const folded = !control.placed || control.form === 'folded';
  const form = measureFormOf(control.form);
  useEffect(() => {
    const el = ref.current;
    if (!el || !measure || folded || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      // Capped, the box is shorter than its content: the content is what it measures.
      const h = Math.round(Math.max(el.getBoundingClientRect().height, scrollOpen ? el.scrollHeight : 0));
      if (h > 0) measure(kind, form, h);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [kind, form, folded, measure, scrollOpen]);
  const capped = scrollOpen && control.form === 'open' && control.height > 0;
  return (
    <div
      ref={ref}
      data-stack-card={kind}
      data-form={control.form ?? ''}
      className={`${folded ? 'hidden' : 'flex'} min-w-0 shrink-0 flex-col ${
        capped ? 'overflow-y-auto overscroll-contain' : ''
      } ${className}`}
      style={{ order: control.order, maxHeight: capped ? control.height : undefined }}
    >
      {children}
    </div>
  );
}

export default ChatOverlayStack;
