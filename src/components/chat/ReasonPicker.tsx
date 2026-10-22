import { useEffect, useEffectEvent, useLayoutEffect, useRef, useState } from 'react';
import type React from 'react';
import { createPortal } from 'react-dom';

import { useOutsidePress } from '../../hooks/useOutsidePress';
import { isImeComposing } from '../../utils/imeComposing';
import { REASON_MAX, reasonKey } from './reasonPick';
import { Tooltip } from '../ui/Tooltip';

const WIDTH = 280;
const EDGE = 8;
const GAP = 6;

const KBD =
  'flex-shrink-0 rounded border border-white/10 bg-white/10 px-1 py-px font-mono text-[9px] leading-none text-white/70';

/** One choice: a menu row like the chat header's flyouts, its key first. */
const ROW =
  'flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-left text-[13px] text-textPrimary transition-colors hover:bg-white/[0.06] focus-visible:bg-white/[0.06] focus-visible:outline-none';

interface ReasonPickerProps {
  /** The dock it opens from; the picker sits under it and follows it. */
  anchor: HTMLElement;
  /** What a pick does, in words ("Ban xqc", "Time out xqc for 10m"). */
  title: string;
  /** From `pickerReasons`. */
  reasons: readonly string[];
  onPick: (reason: string | null) => void;
  onClose: () => void;
}

/**
 * Pick the reason for one ban or timeout at click time: the saved reasons
 * numbered 1-9 (the digit picks), a written one (Enter sends) or none. Esc or
 * a press outside closes it without acting.
 */
export function ReasonPicker({ anchor, title, reasons, onPick, onClose }: ReasonPickerProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState('');
  const anchorRef = useRef<HTMLElement | null>(anchor);
  useLayoutEffect(() => {
    anchorRef.current = anchor;
  }, [anchor]);
  const markInside = useOutsidePress(true, [rootRef, anchorRef], onClose);

  // Under the dock, right edges lined up; above it when there is no room
  // below. Written straight to the node so a scrolling chat re-renders
  // nothing, and closed once the row it belongs to is gone.
  const closeGone = useEffectEvent(() => onClose());
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    let frame = 0;
    const place = () => {
      frame = 0;
      if (!anchor.isConnected) {
        closeGone();
        return;
      }
      const a = anchor.getBoundingClientRect();
      const h = el.offsetHeight;
      const left = Math.min(Math.max(a.right - WIDTH, EDGE), window.innerWidth - WIDTH - EDGE);
      const below = a.bottom + GAP;
      const top = below + h > window.innerHeight - EDGE ? Math.max(EDGE, a.top - GAP - h) : below;
      el.style.left = `${left}px`;
      el.style.top = `${top}px`;
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(place);
    };
    place();
    window.addEventListener('scroll', schedule, true);
    window.addEventListener('resize', schedule);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener('scroll', schedule, true);
      window.removeEventListener('resize', schedule);
    };
  }, [anchor]);

  // Keyboard first: the digits work at once; with nothing saved the field
  // is the only choice, so it takes the focus.
  useEffect(() => {
    if (reasons.length === 0) fieldRef.current?.focus({ preventScroll: true });
    else rootRef.current?.focus({ preventScroll: true });
    // Once, on open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    // Keys pressed here belong to the picker, never to chat shortcuts.
    e.stopPropagation();
    if (isImeComposing(e)) return;
    const result = reasonKey(e.key, {
      inField: e.target === fieldRef.current,
      text,
      reasons,
      modified: e.ctrlKey || e.metaKey || e.altKey,
    });
    if (!result) return;
    if (result.kind === 'type') {
      // The key itself lands in the field once it has the focus.
      fieldRef.current?.focus({ preventScroll: true });
      return;
    }
    e.preventDefault();
    if (result.kind === 'close') onClose();
    else onPick(result.reason);
  };

  return createPortal(
    <div
      ref={rootRef}
      role="dialog"
      aria-label={`${title}: pick a reason`}
      tabIndex={-1}
      data-no-drag="true"
      onKeyDown={onKeyDown}
      onPointerDownCapture={markInside}
      // React carries events out of a portal to the row underneath; a press
      // here must not start a drag or a click on it.
      onPointerDown={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.stopPropagation()}
      // The menu body every glazed control opens (chat header, dock, title bar).
      className="glass-flyout fixed z-[300] flex flex-col rounded-xl p-1.5 outline-none"
      style={{ width: WIDTH, left: -9999, top: -9999 }}
    >
      <div className="flex items-center justify-between gap-2 px-2 pb-1.5 pt-1">
        <span className="truncate text-[10px] font-semibold uppercase tracking-wider text-textMuted">{title}</span>
        <kbd className={KBD}>ESC</kbd>
      </div>

      {reasons.map((reason, i) => (
        <Tooltip key={reason} content={reason}>
          <button
            type="button"
            onClick={() => onPick(reason)}
            className={ROW}
          >
            <kbd className={KBD}>{i + 1}</kbd>
            <span className="min-w-0 flex-1 truncate">{reason}</span>
          </button>
        </Tooltip>
      ))}
      <button type="button" onClick={() => onPick(null)} className={`${ROW} text-textSecondary`}>
        <kbd className={KBD}>0</kbd>
        <span className="min-w-0 flex-1 truncate">No reason</span>
      </button>

      <div className="my-1.5 border-t border-white/[0.06]" />

      <label className="glass-input mx-0.5 mb-0.5 flex items-center gap-2 rounded-lg px-2.5 py-2 focus-within:border-white/20">
        <input
          ref={fieldRef}
          value={text}
          maxLength={REASON_MAX}
          onChange={(e) => setText(e.target.value)}
          placeholder="Write a reason"
          aria-label="Write a reason"
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-textPrimary outline-none placeholder:text-textSecondary/60"
        />
        <kbd className={`${KBD} ${text.trim() ? '' : 'opacity-50'}`}>ENTER</kbd>
      </label>
    </div>,
    document.body,
  );
}
