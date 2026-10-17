import { useEffect } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { Check, RotateCw, AlertTriangle } from 'lucide-react';

export type UpdatePhase = 'installing' | 'installed' | 'error';

/** How long the "Update Installed / Restarting…" card stays up before the app
 *  restarts itself, so the user sees that a restart is happening. */
const RESTART_NOTICE_MS = 2500;

interface UpdateOverlayProps {
  phase: UpdatePhase;
  currentVersion?: string;
  latestVersion?: string;
  /** 0–100 install progress, shown as a number. */
  progressPercent: number;
  /** Human-readable stage line while installing. */
  stageLabel?: string | null;
  errorMessage?: string | null;
  onRestart: () => void;
  onDismiss: () => void;
}

/**
 * Centered update card. Mirrors the staged install → manual restart flow:
 * while the bundle downloads/extracts it shows the percentage, then settles
 * into a green check with a Restart action once the update is staged. Flat
 * Winters' Glass surface (glass-panel bevel, no outer glow).
 */
const UpdateOverlay = ({
  phase,
  currentVersion,
  latestVersion,
  progressPercent,
  stageLabel,
  errorMessage,
  onRestart,
  onDismiss,
}: UpdateOverlayProps) => {
  // Once installed, the app restarts on its own after a short notice — the user
  // is told, not asked. Only the error state can be dismissed; install and the
  // restart notice are not interruptible by a stray backdrop click.
  const canDismiss = phase === 'error';

  // Auto-restart shortly after the update is staged. The card holds for a beat
  // so the restart is visible, then applies it.
  useEffect(() => {
    if (phase !== 'installed') return;
    const t = setTimeout(onRestart, RESTART_NOTICE_MS);
    return () => clearTimeout(t);
  }, [phase, onRestart]);

  return createPortal(
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.18 }}
      className="fixed inset-0 z-[120] flex items-center justify-center bg-black/25"
    >
      <div
        className="absolute inset-0"
        onClick={canDismiss ? onDismiss : undefined}
      />

      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 12 }}
        transition={{ type: 'spring', stiffness: 380, damping: 30 }}
        className="glass-modal relative z-10 w-[420px] max-w-[88vw] max-h-[88vh] px-10 py-14 flex flex-col items-center text-center"
      >
        <AnimatePresence mode="wait">
          {phase === 'installed' ? (
            <motion.div
              key="installed"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex flex-col items-center"
            >
              <motion.div
                initial={{ scale: 0.4, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                transition={{ type: 'spring', stiffness: 420, damping: 18 }}
                className="w-16 h-16 rounded-full flex items-center justify-center mb-5"
                style={{ backgroundColor: '#54d978' }}
              >
                <Check size={32} strokeWidth={3} className="text-black/90" />
              </motion.div>

              <h2 className="text-lg font-bold text-textPrimary">Update Installed</h2>
              <p className="text-sm text-textSecondary mt-1.5">
                Restarting StreamNook to apply the changes…
              </p>

              <div className="flex items-center gap-2 mt-6 text-xs text-textMuted">
                <RotateCw size={14} className="animate-spin" />
                <span>Restarting…</span>
              </div>
            </motion.div>
          ) : phase === 'error' ? (
            <motion.div
              key="error"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex flex-col items-center"
            >
              <div
                className="w-16 h-16 rounded-full flex items-center justify-center mb-5"
                style={{ backgroundColor: 'rgba(239, 68, 68, 0.15)' }}
              >
                <AlertTriangle size={30} style={{ color: 'var(--color-error)' }} />
              </div>

              <h2 className="text-lg font-bold text-textPrimary">Update Failed</h2>
              <p className="text-sm text-textSecondary mt-1.5 break-words max-w-[260px]">
                {errorMessage || 'Something went wrong while updating.'}
              </p>

              <button
                onClick={onDismiss}
                className="glass-button mt-6 px-6 py-2.5 text-sm font-bold text-textPrimary"
              >
                Close
              </button>
            </motion.div>
          ) : (
            <motion.div
              key="installing"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex flex-col items-center w-full"
            >
              <div
                className="w-16 h-16 rounded-full flex items-center justify-center mb-5"
                style={{ backgroundColor: 'rgba(255,255,255,0.04)' }}
              >
                <RotateCw size={28} className="text-textSecondary animate-spin" />
              </div>

              <h2 className="text-lg font-bold text-textPrimary">Installing Update</h2>
              {/* Equal versions: a portable copy moving to the installed app,
                  which is the same version installed properly, not an update. */}
              {currentVersion && latestVersion && currentVersion !== latestVersion && (
                <div className="flex items-center gap-2 text-xs mt-1.5">
                  <span className="text-textMuted">v{currentVersion}</span>
                  <span className="text-textMuted">→</span>
                  <span className="text-textSecondary font-medium">v{latestVersion}</span>
                </div>
              )}

              <div className="mt-5 text-3xl font-bold text-accent tabular-nums">
                {progressPercent}%
              </div>
              <p className="text-[11px] text-textMuted mt-1.5 truncate max-w-full">
                {stageLabel || 'Starting update…'}
              </p>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </motion.div>,
    document.body,
  );
};

export default UpdateOverlay;
