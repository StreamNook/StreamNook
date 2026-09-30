import { AnimatePresence, motion } from 'framer-motion';
import { X } from 'lucide-react';
import { createPortal } from 'react-dom';
import { useAppStore } from '../../stores/AppStore';

// Minimal toast surface for the standalone MultiChat popout. The full ToastManager
// also subscribes to live-stream Tauri events, so mounting it here would double the
// "went live" notifications the main window already shows. Each Tauri window has its
// own store instance, so this paints only THIS window's addToast() queue. Auto-
// dismiss is handled by the store's own timer; this just renders + offers manual
// dismiss.
const TYPE_BORDER: Record<string, string> = {
  error: 'border-error/40',
  success: 'border-green-500/40',
  warning: 'border-amber-500/40',
  info: 'border-borderSubtle',
};

export default function MultiChatToasts() {
  const toasts = useAppStore((s) => s.toasts);
  const removeToast = useAppStore((s) => s.removeToast);
  if (toasts.length === 0) return null;
  return createPortal(
    <div className="pointer-events-none fixed right-4 top-4 z-[300] flex w-[340px] flex-col gap-2">
      <AnimatePresence>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.16 }}
            className={`glass-panel pointer-events-auto flex items-start gap-2 rounded-lg border ${
              TYPE_BORDER[t.type] ?? 'border-borderSubtle'
            } px-3 py-2 shadow-lg`}
          >
            <div className="min-w-0 flex-1 text-xs leading-relaxed text-textPrimary">{t.message}</div>
            <button
              type="button"
              onClick={() => removeToast(t.id)}
              className="shrink-0 text-textSecondary transition-colors hover:text-textPrimary"
              aria-label="Dismiss"
            >
              <X size={14} />
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>,
    document.body,
  );
}
