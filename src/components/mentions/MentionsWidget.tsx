// The main window's Mentions screen, opened from the title bar beside
// Whispers. Opening a row closes it and takes the main chat to the message.

import { useEffect } from 'react';
import { motion } from 'framer-motion';
import { X } from 'lucide-react';
import { MentionsList } from './MentionsList';
import { useMentionInboxStore } from '../../stores/mentionInboxStore';
import { openMentionInMain } from '../../stores/mentionJumpStore';
import type { MentionEntry } from '../../types';

export function MentionsWidget() {
  const open = useMentionInboxStore((s) => s.open);
  const setOpen = useMentionInboxStore((s) => s.setOpen);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setOpen(false); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, setOpen]);

  if (!open) return null;

  const close = () => setOpen(false);
  const openEntry = (entry: MentionEntry) => {
    close();
    void openMentionInMain(entry.id, entry.open_key || entry.room);
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-md"
      onClick={close}
    >
      <motion.div
        initial={{ scale: 0.95, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.95, opacity: 0 }}
        transition={{ type: 'spring', stiffness: 300, damping: 30 }}
        className="glass-panel flex h-[650px] max-h-[calc(100vh-96px)] w-[560px] max-w-[calc(100vw-32px)] flex-col overflow-hidden bg-surface/50 shadow-[0_20px_50px_rgba(0,0,0,0.5)] !backdrop-blur-3xl"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="Mentions"
      >
        <MentionsList
          onOpen={openEntry}
          headerEnd={
            <button
              type="button"
              aria-label="Close"
              onClick={close}
              className="ml-1 flex h-8 w-8 items-center justify-center rounded-lg text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
            >
              <X size={18} />
            </button>
          }
        />
      </motion.div>
    </motion.div>
  );
}

export default MentionsWidget;
