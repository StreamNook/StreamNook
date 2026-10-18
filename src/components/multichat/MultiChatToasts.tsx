import { AnimatePresence, motion } from 'framer-motion';
import { AtSign, Award, Download, MessageCircle, Radio, X } from 'lucide-react';
import { Gift, Package, Sparkle } from 'phosphor-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore, type Toast } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import {
  MIRRORED_NOTIFICATION_EVENT,
  mirroredNoticeHoldMs,
  type MirroredNotice,
  type MirroredNoticeKind,
} from '../../utils/notificationMirror';

// Minimal toast surface for the standalone MultiChat popout. The full ToastManager
// also subscribes to live-stream Tauri events, so mounting it here would double the
// "went live" notifications the main window already shows. Each Tauri window has its
// own store instance, so this paints only THIS window's addToast() queue. Plain
// toasts are dismissed by the store's own timer; a toast with an action (Undo)
// carries no store timer, so this runs it and holds it while hovered.
//
// It also shows the main window's notifications while that window is hidden or
// minimized: Rust sends each one to a single MultiChat window on screen
// (commands/notification_mirror.rs), and a click opens the main window, which
// runs the notification's own action. The ping already came from the main
// window, so these play no sound.
const TYPE_BORDER: Record<string, string> = {
  error: 'border-error/40',
  success: 'border-green-500/40',
  warning: 'border-amber-500/40',
  info: 'border-borderSubtle',
};

/** Cards on screen at once; a burst keeps the newest. */
const MAX_MIRRORED = 3;

// Same glyphs and colours as the notification centre, unboxed.
const KIND_GLYPH: Record<MirroredNoticeKind, React.ReactNode> = {
  live: <Radio size={18} className="text-red-400" />,
  whisper: <MessageCircle size={18} className="text-purple-400" />,
  update: <Download size={18} className="text-yellow-400" />,
  drops: <Package size={18} className="text-green-400" />,
  channel_points: (
    <svg width="18" height="18" viewBox="0 0 24 24" className="text-orange-400" fill="currentColor">
      <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
      <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
    </svg>
  ),
  badge: <Award size={18} className="text-cyan-400" />,
  gift_sub: <Gift size={18} className="text-pink-400" />,
  twitch_reward: <Award size={18} className="text-cyan-400" />,
  membership_gift: <Sparkle size={18} className="text-amber-400" weight="fill" />,
  mention: <AtSign size={18} className="text-accent" />,
};

function MirroredCard({ notice, onDone }: { notice: MirroredNotice; onDone: (id: string) => void }) {
  const [imageFailed, setImageFailed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clickable = !!notice.actionLabel;

  // Hovering holds the card; leaving gives it a fresh hold, so a viewer
  // reading it never has it vanish under the cursor.
  const arm = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => onDone(notice.id), mirroredNoticeHoldMs(notice.kind));
  }, [notice.id, notice.kind, onDone]);
  const hold = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  }, []);
  useEffect(() => {
    arm();
    return hold;
  }, [arm, hold]);

  const open = () => {
    onDone(notice.id);
    invoke('open_mirrored_notification', { id: notice.id, action: notice.action ?? null }).catch((e) =>
      Logger.debug('[MultiChat] open_mirrored_notification failed:', e),
    );
  };

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8 }}
      transition={{ duration: 0.16 }}
      onMouseEnter={hold}
      onMouseLeave={arm}
      onClick={clickable ? open : undefined}
      className={`glass-panel pointer-events-auto flex items-center gap-2.5 rounded-lg border border-borderSubtle px-2.5 py-2 shadow-lg transition-colors ${
        clickable ? 'cursor-pointer hover:border-accent/50' : ''
      }`}
    >
      {notice.imageUrl && !imageFailed ? (
        <img
          src={notice.imageUrl}
          alt=""
          onError={() => setImageFailed(true)}
          className={`h-[34px] w-[34px] flex-none object-cover ${notice.roundImage ? 'rounded-full' : 'rounded'}`}
        />
      ) : (
        <span className="sn-notif-glyph">{KIND_GLYPH[notice.kind] ?? null}</span>
      )}
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-semibold leading-snug text-textPrimary">{notice.title}</div>
        {(notice.body || clickable) && (
          <div className="flex items-baseline gap-2 text-xs leading-snug">
            {notice.body && <span className="line-clamp-2 min-w-0 flex-1 text-textSecondary">{notice.body}</span>}
            {clickable && (
              <span className="ml-auto flex-none text-[11px] font-medium text-accent">{notice.actionLabel}</span>
            )}
          </div>
        )}
      </div>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDone(notice.id);
        }}
        className="flex-none self-start text-textSecondary transition-colors hover:text-textPrimary"
        aria-label="Dismiss"
      >
        <X size={14} />
      </button>
    </motion.div>
  );
}

function PopoutToast({ toast, onDone }: { toast: Toast; onDone: (id: number) => void }) {
  // Only toasts the store left without a timer (an action, live) run one here.
  const ownTimer = !toast.timeoutId && toast.duration > 0;
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const remaining = useRef<number | null>(null);
  const startedAt = useRef(0);

  const arm = useCallback(() => {
    if (!ownTimer) return;
    if (timer.current) clearTimeout(timer.current);
    remaining.current ??= Math.max(0, toast.duration - (Date.now() - toast.createdAt));
    startedAt.current = Date.now();
    timer.current = setTimeout(() => onDone(toast.id), remaining.current);
  }, [ownTimer, onDone, toast.id, toast.duration, toast.createdAt]);
  const hold = useCallback(() => {
    if (!timer.current) return;
    clearTimeout(timer.current);
    timer.current = null;
    remaining.current = Math.max(0, (remaining.current ?? 0) - (Date.now() - startedAt.current));
  }, []);
  useEffect(() => {
    arm();
    return hold;
  }, [arm, hold]);

  return (
    <motion.div
      initial={{ opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8 }}
      transition={{ duration: 0.16 }}
      onMouseEnter={hold}
      onMouseLeave={arm}
      className={`glass-panel pointer-events-auto flex items-start gap-2 rounded-lg border ${
        TYPE_BORDER[toast.type] ?? 'border-borderSubtle'
      } px-3 py-2 shadow-lg`}
    >
      <div className="min-w-0 flex-1 text-xs leading-relaxed text-textPrimary">{toast.message}</div>
      {toast.action && (
        <button
          type="button"
          onClick={() => {
            toast.action?.onClick();
            onDone(toast.id);
          }}
          className="shrink-0 rounded px-2 py-0.5 text-[11px] font-medium text-accent transition-colors hover:bg-accent/15"
        >
          {toast.action.label}
        </button>
      )}
      <button
        type="button"
        onClick={() => onDone(toast.id)}
        // Centers the cross on the message's first line.
        className="mt-[3px] shrink-0 text-textSecondary transition-colors hover:text-textPrimary"
        aria-label="Dismiss"
      >
        <X size={14} />
      </button>
    </motion.div>
  );
}

export default function MultiChatToasts() {
  const toasts = useAppStore((s) => s.toasts);
  const removeToast = useAppStore((s) => s.removeToast);
  const [mirrored, setMirrored] = useState<MirroredNotice[]>([]);

  useEffect(() => {
    const unlisten = listen<MirroredNotice>(MIRRORED_NOTIFICATION_EVENT, (event) => {
      setMirrored((prev) => [event.payload, ...prev.filter((n) => n.id !== event.payload.id)].slice(0, MAX_MIRRORED));
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const dropMirrored = useCallback((id: string) => {
    setMirrored((prev) => prev.filter((n) => n.id !== id));
  }, []);

  if (toasts.length === 0 && mirrored.length === 0) return null;
  return createPortal(
    <div className="pointer-events-none fixed right-4 top-4 z-[300] flex w-[min(340px,calc(100vw-32px))] flex-col gap-2">
      <AnimatePresence>
        {mirrored.map((n) => (
          <MirroredCard key={n.id} notice={n} onDone={dropMirrored} />
        ))}
        {toasts.map((t) => (
          <PopoutToast key={t.id} toast={t} onDone={removeToast} />
        ))}
      </AnimatePresence>
    </div>,
    document.body,
  );
}
