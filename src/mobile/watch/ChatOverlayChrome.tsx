// The landscape chat overlay's own controls, kept ON the overlay so nobody
// has to leave the video to find them.
//
// One handle on the edge that faces the picture: drag it to resize the column,
// tap it for the strip. The strip carries the see-through slider and a way to
// hide chat, and puts itself away after a few seconds of being ignored. Both
// changes apply live and are saved on release, so a swipe reads as one motion
// rather than a stutter of settings writes.
//
// The phone's beside-the-video column reuses the handle alone: with no picture
// behind it there is nothing to see through, so it passes no opacity and gets
// no strip.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Drop, X } from 'phosphor-react';
import { hapticTick } from '../ui/haptics';

interface Props {
  /** Which screen edge the overlay sits on; the handle lives on the other. */
  side: 'left' | 'right';
  width: number;
  minWidth: number;
  maxWidth: number;
  onWidthChange: (w: number) => void;
  onWidthCommit: (w: number) => void;
  /** The see-through strip. Omitted, the handle only resizes. */
  strip?: {
    /** 0..100 */
    opacity: number;
    onOpacityChange: (o: number) => void;
    onOpacityCommit: (o: number) => void;
    onHide: () => void;
  };
}

const TAP_SLOP_PX = 5;
const STRIP_IDLE_MS = 4000;
const OPACITY_COMMIT_MS = 350;

export const ChatOverlayChrome: React.FC<Props> = ({
  side,
  width,
  minWidth,
  maxWidth,
  onWidthChange,
  onWidthCommit,
  strip,
}) => {
  // Open on arrival: the strip is the only sign the slider exists, and a
  // control nobody has been shown is a control nobody finds. It puts itself
  // away on the same idle timer as always.
  const hasStrip = strip != null;
  const [stripOpen, setStripOpen] = useState(hasStrip);
  const idle = useRef<ReturnType<typeof setTimeout> | null>(null);
  const commit = useRef<ReturnType<typeof setTimeout> | null>(null);
  const drag = useRef<{ id: number; startX: number; startW: number; lastW: number; moved: boolean } | null>(null);

  // The strip closes on its own once it has been left alone. Every touch on
  // it pushes the deadline back.
  const armIdle = useCallback(() => {
    if (idle.current) clearTimeout(idle.current);
    idle.current = setTimeout(() => setStripOpen(false), STRIP_IDLE_MS);
  }, []);
  useEffect(() => {
    if (stripOpen) armIdle();
    else if (idle.current) clearTimeout(idle.current);
  }, [stripOpen, armIdle]);
  useEffect(
    () => () => {
      if (idle.current) clearTimeout(idle.current);
      if (commit.current) clearTimeout(commit.current);
    },
    [],
  );

  const clampW = useCallback(
    (w: number) => Math.round(Math.max(minWidth, Math.min(maxWidth, w))),
    [minWidth, maxWidth],
  );

  const onHandleDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { id: e.pointerId, startX: e.clientX, startW: width, lastW: width, moved: false };
  };
  const onHandleMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    const dx = e.clientX - d.startX;
    if (!d.moved && Math.abs(dx) > TAP_SLOP_PX) d.moved = true;
    if (!d.moved) return;
    // The handle is on the edge nearest the picture, so on a right-side
    // overlay pulling LEFT makes it wider.
    const w = clampW(side === 'right' ? d.startW - dx : d.startW + dx);
    if (w !== d.lastW) {
      d.lastW = w;
      onWidthChange(w);
    }
  };
  const onHandleEnd = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    drag.current = null;
    if (d.moved) {
      onWidthCommit(d.lastW);
      return;
    }
    // A tap: the strip, where there is one.
    if (!hasStrip) return;
    hapticTick();
    setStripOpen((v) => !v);
  };
  // The OS took the touch (an edge swipe, the shade): neither a resize nor a
  // tap. Any width already applied live is kept as the saved value.
  const onHandleCancel = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    drag.current = null;
    if (d.moved) onWidthCommit(d.lastW);
  };

  const onOpacityInput = (v: number) => {
    if (!strip) return;
    strip.onOpacityChange(v);
    armIdle();
    if (commit.current) clearTimeout(commit.current);
    commit.current = setTimeout(() => strip.onOpacityCommit(v), OPACITY_COMMIT_MS);
  };

  return (
    <>
      {/* The handle straddles the inner edge: half over the picture so it
          reads as the column's edge rather than a button inside it, with a
          hit area wider than the pill it draws. */}
      <div
        className={`absolute top-1/2 -translate-y-1/2 z-40 w-7 h-20 flex items-center justify-center ${
          side === 'right' ? 'left-0 -translate-x-1/2' : 'right-0 translate-x-1/2'
        }`}
        style={{ touchAction: 'none' }}
        role="slider"
        aria-label="Chat width"
        aria-valuemin={minWidth}
        aria-valuemax={maxWidth}
        aria-valuenow={width}
        onPointerDown={onHandleDown}
        onPointerMove={onHandleMove}
        onPointerUp={onHandleEnd}
        onPointerCancel={onHandleCancel}
      >
        {/* Quiet: a hairline the eye can find when it looks for an edge,
            not a control competing with the video. The hit area around it
            is what makes it grabbable. */}
        <div className="w-[3px] h-8 rounded-full bg-white/35" />
      </div>

      <AnimatePresence>
        {strip && stripOpen && (
          <motion.div
            className="absolute bottom-0 left-0 right-0 z-40 p-2"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 8 }}
            transition={{ duration: 0.16 }}
            onPointerDown={armIdle}
          >
            <div className="chrome-glaze chrome-glaze--flat h-11 px-3 flex items-center gap-2.5 text-white">
              <Drop size={17} weight="fill" className="shrink-0 opacity-80" />
              <input
                type="range"
                min={0}
                max={100}
                step={5}
                value={strip.opacity}
                onChange={(e) => onOpacityInput(Number(e.target.value))}
                aria-label="Chat background"
                className="flex-1 min-w-0 accent-accent"
                style={{ touchAction: 'none' }}
              />
              <span className="text-[12px] font-semibold tabular-nums w-9 text-right">{strip.opacity}%</span>
              <button
                onClick={strip.onHide}
                aria-label="Hide chat"
                className="sn-touch -mr-2 flex items-center justify-center w-9 h-9 rounded-full active:bg-white/10"
              >
                <X size={17} weight="bold" />
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </>
  );
};
