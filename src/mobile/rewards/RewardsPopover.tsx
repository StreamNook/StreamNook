// A small glass popover anchored to the control that opened it.
//
// Touch has no hover, so what a desktop tooltip carried (the catch-up notes,
// an earn step's long form) opens here on a tap, and the badge wall's five
// sorts open here as a menu instead of a capsule that scrolled the last one
// off screen.
//
// Portaled to <body> and placed from the anchor's rect, so the scrolling wall
// cannot clip it. A transparent full-screen backdrop takes the dismissing tap
// the way a system menu does, so tapping away never also opens the tile under
// the finger. It joins the navStore sheet stack so Android back closes it
// before anything behind it.
import React, { useEffect, useId, useLayoutEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { useMobileNavStore } from '../navStore';

// Space between the anchor and the popover, and the least it keeps from the
// screen's side edges.
const GAP = 6;
const EDGE = 12;
// The floating tab bar and its lift off the bottom edge: a popover that would
// run under it opens above its anchor instead.
const BOTTOM_CLEAR = 96;
const TOP_CLEAR = 24;

interface RewardsPopoverProps {
  open: boolean;
  onClose: () => void;
  anchor: React.RefObject<HTMLElement | null>;
  /** Fixed, so the side clamp is known before the popover is measured. */
  width: number;
  /** Which anchor edge the popover lines up with. */
  align?: 'start' | 'end';
  role: 'menu' | 'dialog';
  /** Spoken name of the popover. */
  label: string;
  children: React.ReactNode;
}

type SurfaceProps = Omit<RewardsPopoverProps, 'open'>;

const Surface: React.FC<SurfaceProps> = ({ onClose, anchor, width, align = 'start', role, label, children }) => {
  const panel = useRef<HTMLDivElement>(null);

  // Placed before the first paint, straight on the node: the position comes
  // from the DOM, so a state round trip would only add a frame drawn in the
  // wrong place.
  useLayoutEffect(() => {
    const el = panel.current;
    const at = anchor.current?.getBoundingClientRect();
    if (!el || !at) return;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const wanted = align === 'end' ? at.right - width : at.left;
    const left = Math.min(Math.max(wanted, EDGE), vw - width - EDGE);
    const height = el.offsetHeight;
    const below = at.bottom + GAP;
    const above = below + height > vh - BOTTOM_CLEAR && at.top - GAP - height >= TOP_CLEAR;
    el.style.left = `${left}px`;
    el.style.top = `${above ? at.top - GAP - height : below}px`;
    // Grows out of the corner nearest the anchor, so it reads as coming from it.
    el.style.transformOrigin = `${align === 'end' ? 'right' : 'left'} ${above ? 'bottom' : 'top'}`;
    // The chosen item, or the panel, takes focus so a screen reader lands
    // inside; it goes back to the anchor on close.
    const target = el.querySelector<HTMLElement>('[aria-checked="true"]') ?? el;
    target.focus({ preventScroll: true });
    const opener = anchor.current;
    return () => opener?.focus({ preventScroll: true });
  }, [anchor, align, width]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    // A motion root so AnimatePresence holds the backdrop (still taking taps)
    // until the panel's exit has played.
    <motion.div className="fixed inset-0 z-[9000]">
      <div className="absolute inset-0" onClick={onClose} aria-hidden />
      <motion.div
        ref={panel}
        role={role}
        aria-label={label}
        tabIndex={-1}
        // The phone's one menu surface, near-opaque and solid at Glassiness 0,
        // for the sort menu and the tap-open notes alike.
        className="sn-menu absolute outline-none"
        style={{ width }}
        initial={{ opacity: 0, scale: 0.94 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.94 }}
        transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
      >
        {children}
      </motion.div>
    </motion.div>
  );
};

export const RewardsPopover: React.FC<RewardsPopoverProps> = ({ open, onClose, ...rest }) => {
  const popoverId = useId();
  const pushSheet = useMobileNavStore((s) => s.pushSheet);
  const popSheet = useMobileNavStore((s) => s.popSheet);

  useEffect(() => {
    if (!open) return;
    pushSheet(popoverId);
    const onBackClose = (e: Event) => {
      if ((e as CustomEvent<string>).detail === popoverId) onClose();
    };
    window.addEventListener('sn:close-sheet', onBackClose);
    return () => {
      window.removeEventListener('sn:close-sheet', onBackClose);
      popSheet(popoverId);
    };
  }, [open, popoverId, pushSheet, popSheet, onClose]);

  return createPortal(
    <AnimatePresence>{open && <Surface key="popover" onClose={onClose} {...rest} />}</AnimatePresence>,
    document.body,
  );
};
