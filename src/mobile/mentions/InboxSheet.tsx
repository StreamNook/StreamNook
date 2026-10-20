// The inbox sheet: it grows up out of the message bar from the tray's corner,
// flush on the bar, over the lower two thirds of the chat; the chat above it
// stays live. It closes the way it came on a slide down, a press anywhere
// outside it, the tray (now an X), focusing the message field, or back.
//
// One spring for both directions, the desktop island's. Only transform and
// opacity move, so the chat behind keeps its frame rate.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { useMobileNavStore } from '../navStore';
import { closeInboxSheet, useMentionsStore, type InboxSurface } from './mentionInbox';
import { InboxPanel } from './InboxPanel';
import { INBOX_SPRING } from './inboxText';

const DISMISS_DISTANCE = 90;

const SheetBody: React.FC<{ side: 'left' | 'right'; bottom: number; onClose: () => void }> = ({ side, bottom, onClose }) => {
  const ref = useRef<HTMLDivElement>(null);
  const [dragY, setDragY] = useState(0);
  const [dragging, setDragging] = useState(false);
  const start = useRef<{ id: number; y: number; t: number; last: number; v: number } | null>(null);
  const onPress = useOutsidePress(true, [ref, '[data-inbox-tray]'], onClose);

  const headerHandlers: React.HTMLAttributes<HTMLDivElement> = {
    onPointerDown: (e) => {
      if ((e.target as HTMLElement).closest('button')) return;
      start.current = { id: e.pointerId, y: e.clientY, t: e.timeStamp, last: 0, v: 0 };
    },
    onPointerMove: (e) => {
      const s = start.current;
      if (!s || s.id !== e.pointerId) return;
      const dy = Math.max(0, e.clientY - s.y);
      if (!dragging && dy < 6) return;
      if (!dragging) {
        setDragging(true);
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      }
      s.v = (dy - s.last) / Math.max(1, e.timeStamp - s.t);
      s.last = dy;
      s.t = e.timeStamp;
      setDragY(dy);
    },
    onPointerUp: (e) => {
      const s = start.current;
      start.current = null;
      if (!s || s.id !== e.pointerId || !dragging) return;
      setDragging(false);
      if (s.last > DISMISS_DISTANCE || s.v > 0.6) onClose();
      else setDragY(0);
    },
    onPointerCancel: () => {
      start.current = null;
      setDragging(false);
      setDragY(0);
    },
  };

  return (
    <motion.div
      ref={ref}
      onPointerDownCapture={onPress}
      className={`absolute z-40 h-[68%] min-h-[220px] max-h-full ${bottom ? 'left-2 right-2' : 'left-2.5 right-2.5'}`}
      style={{ bottom, transformOrigin: side === 'left' ? '20px 100%' : 'calc(100% - 20px) 100%' }}
      initial={{ opacity: 0, scale: 0.12 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.12 }}
      transition={INBOX_SPRING}
    >
      <div
        className={`chrome-glaze chrome-glaze--frosted sn-inbox-sheet h-full overflow-hidden ${bottom ? '' : 'sn-inbox-sheet--flush'}`}
        style={{
          borderRadius: bottom ? 20 : '20px 20px 0 0',
          transform: dragY ? `translateY(${dragY}px)` : undefined,
          transition: dragging ? 'none' : 'transform 0.22s cubic-bezier(0.2, 0.8, 0.2, 1)',
        }}
      >
        <motion.div
          className="h-full"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15, delay: 0.05 }}
        >
          <InboxPanel variant="sheet" onClose={onClose} headerHandlers={headerHandlers} />
        </motion.div>
      </div>
    </motion.div>
  );
};

/** `bottom`: what it rises from. 0 sits it on the message bar; the landscape
 *  column has no bar, so it rises from the strip's top instead. */
export const InboxSheet: React.FC<{ surface: InboxSurface; side: 'left' | 'right'; bottom?: number }> = ({
  surface,
  side,
  bottom = 0,
}) => {
  const open = useMentionsStore((s) => s.sheetIn === surface);
  const sheetId = useId();
  const pushSheet = useMobileNavStore((s) => s.pushSheet);
  const popSheet = useMobileNavStore((s) => s.popSheet);
  const close = useCallback(() => closeInboxSheet(), []);

  // Back closes it, like every sheet.
  useEffect(() => {
    if (!open) return;
    pushSheet(sheetId);
    const onBack = (e: Event) => {
      if ((e as CustomEvent<string>).detail === sheetId) close();
    };
    window.addEventListener('sn:close-sheet', onBack);
    return () => {
      window.removeEventListener('sn:close-sheet', onBack);
      popSheet(sheetId);
    };
  }, [open, sheetId, pushSheet, popSheet, close]);

  return <AnimatePresence>{open && <SheetBody key="inbox" side={side} bottom={bottom} onClose={close} />}</AnimatePresence>;
};
