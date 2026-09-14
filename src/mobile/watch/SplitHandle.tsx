// Drag handle between player and chat on a screen big enough for both.
//
// The desktop has mouse-driven resize handles on its docked chat; this is the
// touch equivalent, and it exists because "fill the width" is the wrong rule on
// a near-square screen. A 1280dp-wide tablet at 16:9 spends 720 of its 800dp on
// video, which leaves room for the composer and nothing else. The right split
// between picture size and how much chat you can read is a preference, not
// something geometry can decide.
//
// The video never distorts: it is `object-contain`, so shrinking its pane just
// adds bars on whichever axis has the surplus.
import React, { useCallback, useRef } from 'react';
import { hapticStep, hapticTick } from '../ui/haptics';

interface Props {
  /** `x` when chat sits beside the player, `y` when it sits below. */
  axis: 'x' | 'y';
  /** Reports the new chat fraction (0..1) of the container's long axis. */
  onDrag: (frac: number) => void;
  /** Container length along the axis, in CSS px. */
  length: number;
  /** Smallest chat and player sizes, in CSS px. */
  minChat: number;
  minPlayer: number;
  /** Hinge centre in CSS px, when one crosses along this axis. */
  snapAt?: number | null;
  /** Tapping the seam without dragging returns to the natural split. */
  onReset: () => void;
}

/** Within this many px of the hinge, the split takes the seam exactly. */
const SNAP_PX = 20;

export const SplitHandle: React.FC<Props> = ({
  axis,
  onDrag,
  length,
  minChat,
  minPlayer,
  snapAt,
  onReset,
}) => {
  const snapped = useRef(false);

  const apply = useCallback(
    (clientPos: number, origin: number) => {
      // Position of the divider measured from the container's start edge.
      let playerSize = clientPos - origin;

      if (snapAt != null && Math.abs(playerSize - snapAt) <= SNAP_PX) {
        // One tick as it takes the seam, so the snap is felt rather than
        // only seen. Repeats are suppressed while it stays snapped.
        if (!snapped.current) {
          snapped.current = true;
          hapticTick();
        }
        playerSize = snapAt;
      } else {
        snapped.current = false;
      }

      const clamped = Math.max(minPlayer, Math.min(length - minChat, playerSize));
      onDrag((length - clamped) / length);
    },
    [length, minChat, minPlayer, onDrag, snapAt],
  );

  const moved = useRef(false);

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLElement>) => {
      const el = e.currentTarget;
      const parent = el.parentElement;
      if (!parent) return;
      const rect = parent.getBoundingClientRect();
      const origin = axis === 'x' ? rect.left : rect.top;

      el.setPointerCapture(e.pointerId);
      hapticStep();

      const start = axis === 'x' ? e.clientX : e.clientY;
      moved.current = false;

      const move = (ev: PointerEvent) => {
        // Chromium commits to scrolling at touchstart, so `touch-action: none`
        // has to be on the handle statically; that is what makes this drag
        // possible at all inside a scrollable chat column.
        ev.preventDefault();
        const pos = axis === 'x' ? ev.clientX : ev.clientY;
        if (Math.abs(pos - start) > 4) moved.current = true;
        apply(pos, origin);
      };
      const up = (ev: PointerEvent) => {
        el.releasePointerCapture(ev.pointerId);
        el.removeEventListener('pointermove', move);
        el.removeEventListener('pointerup', up);
        el.removeEventListener('pointercancel', up);
        snapped.current = false;
      };
      el.addEventListener('pointermove', move);
      el.addEventListener('pointerup', up);
      el.addEventListener('pointercancel', up);
    },
    [apply, axis],
  );

  const horizontal = axis === 'y';

  return (
    // A real <button>, not a div, and that is load-bearing rather than
    // cosmetic. Chromium's TOUCH ADJUSTMENT only considers nodes it treats as
    // clickable when it resolves a touch, and it snaps to the best candidate in
    // the region. A bare div carrying only a pointer handler is not a
    // candidate, so every touch on the divider was awarded to the full-height
    // chat row beside it and the drag silently never began. `elementFromPoint`
    // does not model any of this, which is why the console insisted the handle
    // was being hit while a finger disagreed.
    <button
      type="button"
      onPointerDown={onPointerDown}
      onClick={() => {
        // A drag ends in a click too; only a genuine tap should reset.
        if (moved.current) {
          moved.current = false;
          return;
        }
        onReset();
      }}
      className={`shrink-0 relative flex items-center justify-center touch-none ${
        horizontal ? 'w-full h-4 cursor-row-resize' : 'h-full w-4 cursor-col-resize'
      }`}
      aria-label="Resize player and chat, tap to reset"
    >
      {/* The hit area also has to be bigger than the seam looks, so the finger
          has ~40px to land on while the layout seam stays thin. */}
      <div
        className={`absolute touch-none ${
          horizontal ? 'inset-x-0 -inset-y-3' : 'inset-y-0 -inset-x-3'
        }`}
      />
      <div
        className={`rounded-full bg-textMuted/40 pointer-events-none ${
          horizontal ? 'w-9 h-1' : 'h-9 w-1'
        }`}
      />
    </button>
  );
};
