// A new mention, reply or highlight, as a compact card risen out of the tray
// just above the message bar: who, where, and a line of what they said. It
// stays a few seconds and folds back into the tray's count. A tap jumps to the
// message (the keyboard and the draft stay as they were); a slide down puts it
// away early. While the composer holds a long draft or its suggestions, the
// card waits for it, so it never sits over what is being written.
import React, { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { dismissArrival, openMention, useMentionsStore } from './mentionInbox';
import { InboxFace, InboxMessage } from './InboxParts';
import { INBOX_SPRING, whereWords } from './inboxText';

/** How long a card stays up before it folds into the count. */
export const ARRIVAL_MS = 4000;

export const InboxArrival: React.FC<{ side: 'left' | 'right' }> = ({ side }) => {
  const arrival = useMentionsStore((s) => s.arrival);
  const busy = useMentionsStore((s) => s.composerBusy);
  const sheetOpen = useMentionsStore((s) => s.sheetIn !== null);
  const shown = !!arrival && !busy && !sheetOpen;
  const seq = arrival?.seq ?? 0;
  const [dragY, setDragY] = useState(0);
  const drag = useRef<{ id: number; y: number } | null>(null);
  const moved = useRef(false);

  // The few seconds start once the card is actually up; a hold restarts them.
  useEffect(() => {
    if (!shown) return;
    const t = window.setTimeout(() => dismissArrival(seq), ARRIVAL_MS);
    return () => window.clearTimeout(t);
  }, [shown, seq]);

  const entry = arrival?.entry;
  const where = entry ? whereWords(entry) : null;
  return (
    <AnimatePresence>
      {shown && entry && where && (
        <motion.button
          key={seq}
          type="button"
          className="absolute left-2.5 right-2.5 bottom-2 z-40 text-left"
          style={{ transformOrigin: side === 'left' ? '20px 100%' : 'calc(100% - 20px) 100%' }}
          initial={{ opacity: 0, scale: 0.2, y: 10 }}
          animate={{ opacity: 1, scale: 1, y: dragY }}
          exit={{ opacity: 0, scale: 0.2, y: 14 }}
          transition={INBOX_SPRING}
          // Held on the press, so a field that has the keyboard keeps it.
          onPointerDown={(e) => {
            e.preventDefault();
            drag.current = { id: e.pointerId, y: e.clientY };
            moved.current = false;
          }}
          onPointerMove={(e) => {
            const d = drag.current;
            if (!d || d.id !== e.pointerId) return;
            const dy = Math.max(0, e.clientY - d.y);
            if (dy > 6) moved.current = true;
            setDragY(dy);
          }}
          onPointerUp={() => {
            drag.current = null;
            if (dragY > 24) dismissArrival(seq);
            setDragY(0);
          }}
          onPointerCancel={() => {
            drag.current = null;
            setDragY(0);
          }}
          onClick={() => {
            if (moved.current) return;
            dismissArrival(seq);
            void openMention(entry);
          }}
          aria-label={`${entry.sender_name || entry.sender_login} ${where.lead} ${where.place ?? ''}: ${entry.text}. Open`}
        >
          <span className="chrome-glaze chrome-glaze--frosted sn-inbox-arrival flex items-center gap-2.5 pl-2 pr-3 h-[54px] rounded-[18px]">
            <InboxFace entry={entry} avatar={arrival?.avatar ?? null} size={36} />
            <span className="flex-1 min-w-0 flex flex-col">
              <span className="flex items-baseline gap-1.5 min-w-0 leading-[18px]">
                <span
                  className="text-[14px] font-bold truncate text-textPrimary"
                  style={entry.sender_color ? { color: entry.sender_color } : undefined}
                >
                  {entry.sender_name || entry.sender_login}
                </span>
                <span className="shrink-0 text-[12.5px] text-textMuted">
                  {where.lead}
                  {where.place ? ` ${where.place}` : ''}
                </span>
                <span className="flex-1" />
                <span className="shrink-0 text-[11.5px] text-textMuted tabular-nums">now</span>
              </span>
              <span className="sn-inbox-fade-end text-[13.5px] leading-[20px] text-textSecondary overflow-hidden whitespace-nowrap">
                <InboxMessage entry={entry} emote={19} />
              </span>
            </span>
          </span>
        </motion.button>
      )}
    </AnimatePresence>
  );
};
