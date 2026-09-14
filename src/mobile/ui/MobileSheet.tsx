// Bottom sheet: the universal mobile replacement for desktop popovers,
// dropdowns, and context menus. Glass design system, drag-to-dismiss, backdrop
// tap, and Android back integration via the navStore sheet stack.
//
// Motion: the sheet slides up and the backdrop fades, both fast. framer-motion
// rather than CSS because the exit half needs the element to outlive its own
// unmount, which AnimatePresence handles and a CSS transition cannot. The
// app-wide MotionScope already supplies framer's reducedMotion, so the Motion
// setting governs this for free.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { useMobileNavStore } from '../navStore';

export interface MobileSheetProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  children: React.ReactNode;
  /** Max sheet height as a viewport fraction. Default 0.72. */
  maxHeightFraction?: number;
}

const DISMISS_DISTANCE = 96;

// Stiff and well damped: quick and snappy without the wobble an underdamped
// spring gives.
const SETTLE = { type: 'spring', stiffness: 560, damping: 42, mass: 0.7 } as const;

// Inner surface owns the drag state; it unmounts when the sheet closes, so
// drag position resets naturally without any state-sync effect.
const SheetSurface: React.FC<Omit<MobileSheetProps, 'open'>> = ({
  onClose,
  title,
  children,
  maxHeightFraction = 0.72,
}) => {
  const [dragY, setDragY] = useState(0);
  // Mirrors dragStart as STATE because the transition is chosen during render: a
  // finger drag must track 1:1 with no easing, a release should spring. Reading
  // the ref in render would not re-render when the drag begins.
  const [dragging, setDragging] = useState(false);
  const dragStart = useRef<number | null>(null);

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    dragStart.current = e.clientY;
    setDragging(true);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const onPointerMove = useCallback((e: React.PointerEvent) => {
    if (dragStart.current === null) return;
    setDragY(Math.max(0, e.clientY - dragStart.current));
  }, []);

  const onPointerUp = useCallback(() => {
    if (dragStart.current === null) return;
    dragStart.current = null;
    setDragging(false);
    setDragY((y) => {
      if (y > DISMISS_DISTANCE) onClose();
      return 0;
    });
  }, [onClose]);

  return (
    // Root is a motion element so AnimatePresence waits for this whole subtree's
    // exit before unmounting it.
    <motion.div
      className="fixed inset-0 z-[9000]"
      role="dialog"
      aria-modal="true"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.13, ease: 'linear' }}
    >
      <div
        className="absolute inset-0 bg-black/50"
        style={{ backdropFilter: 'blur(6px)', WebkitBackdropFilter: 'blur(6px)' }}
        onClick={onClose}
      />
      <motion.div
        // Capped and centred rather than edge to edge: a sheet stretched across a
        // tablet or an unfolded Fold puts its controls a hand-span apart.
        className="absolute inset-x-0 bottom-0 mx-auto w-full max-w-[640px] glass-modal rounded-b-none flex flex-col"
        style={{
          maxHeight: `calc(100dvh * ${maxHeightFraction})`,
          paddingBottom: 'var(--sn-safe-b, 0px)',
        }}
        initial={{ y: '100%' }}
        animate={{ y: dragY }}
        exit={{ y: '100%' }}
        transition={dragging ? { duration: 0 } : SETTLE}
      >
        <div
          className="flex flex-col items-center pt-2.5 pb-1 shrink-0 touch-none"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        >
          <div className="w-9 h-1 rounded-full bg-textMuted/40" />
          {title && <div className="mt-2 text-sm font-semibold text-textPrimary">{title}</div>}
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto px-4 pb-4">{children}</div>
      </motion.div>
    </motion.div>
  );
};

export const MobileSheet: React.FC<MobileSheetProps> = ({ open, onClose, ...rest }) => {
  const sheetId = useId();
  const pushSheet = useMobileNavStore((s) => s.pushSheet);
  const popSheet = useMobileNavStore((s) => s.popSheet);

  // Register in the Android back-button stack while open (external store sync).
  useEffect(() => {
    if (!open) return;
    pushSheet(sheetId);
    const onBackClose = (e: Event) => {
      if ((e as CustomEvent<string>).detail === sheetId) onClose();
    };
    window.addEventListener('sn:close-sheet', onBackClose);
    return () => {
      window.removeEventListener('sn:close-sheet', onBackClose);
      popSheet(sheetId);
    };
  }, [open, sheetId, pushSheet, popSheet, onClose]);

  // The portal stays mounted so AnimatePresence has somewhere to play the exit.
  // An empty portal container costs nothing.
  return createPortal(
    <AnimatePresence>
      {open && <SheetSurface key="sheet" onClose={onClose} {...rest} />}
    </AnimatePresence>,
    document.body,
  );
};
