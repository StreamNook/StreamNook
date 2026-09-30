import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Loader2, Clapperboard, Play } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';

// Twitch clip length limits.
const MIN_LEN = 5;
const MAX_LEN = 60;

interface EditSession {
  raw_media_id: string;
  duration_seconds: number;
  preview_url: string;
}

function fmt(t: number): string {
  const s = Math.max(0, t);
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${sec.toString().padStart(2, '0')}`;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

// The clip trim editor (VOD or live broadcast). Captures a ~90s raw-media window
// (begin_clip_edit), plays that signed footage, and lets the user set an exact
// in/out segment + title before finalizing (finalize_clip → the play modal).
export default function ClipEditor() {
  const session = useAppStore((s) => s.clipEditor);
  const close = useAppStore((s) => s.closeClipEditor);

  return (
    <AnimatePresence>
      {session && (
        <ClipEditorInner
          key={`${session.vodId ?? session.broadcastId}:${session.offsetSeconds}`}
          vodId={session.vodId}
          broadcastId={session.broadcastId}
          offsetSeconds={session.offsetSeconds}
          channelName={session.channelName}
          onClose={close}
        />
      )}
    </AnimatePresence>
  );
}

function ClipEditorInner({
  vodId,
  broadcastId,
  offsetSeconds,
  channelName,
  onClose,
}: {
  vodId?: string;
  broadcastId?: string;
  offsetSeconds: number;
  channelName: string;
  onClose: () => void;
}) {
  const addToast = useAppStore((s) => s.addToast);
  const openClipModal = useAppStore((s) => s.openClipModal);

  const [edit, setEdit] = useState<EditSession | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [title, setTitle] = useState('');
  const [start, setStart] = useState(0);
  const [end, setEnd] = useState(0);
  const [creating, setCreating] = useState(false);
  const [currentTime, setCurrentTime] = useState(0); // video playhead, for the marker

  const videoRef = useRef<HTMLVideoElement | null>(null);
  const trackRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ kind: 'start' | 'end' | 'region'; startX: number; s: number; e: number } | null>(
    null,
  );

  const duration = edit?.duration_seconds ?? 0;

  // 1. Capture the footage window.
  useEffect(() => {
    // Inner is keyed per session, so state starts fresh — no reset needed here.
    let cancelled = false;
    invoke<EditSession>('begin_clip_edit', { vodId, broadcastId, offsetSeconds })
      .then((s) => {
        if (cancelled) return;
        setEdit(s);
        // Default to the last 30s of the window (the moment you were watching).
        const len = Math.min(30, Math.max(MIN_LEN, s.duration_seconds));
        const st = Math.max(0, s.duration_seconds - len);
        setStart(st);
        setEnd(st + len);
      })
      .catch((e) => {
        if (cancelled) return;
        Logger.error('[ClipEditor] begin_clip_edit failed:', e);
        setLoadError(String(e).replace(/^Error:\s*/, '') || 'unknown error');
      });
    return () => {
      cancelled = true;
    };
  }, [vodId, broadcastId, offsetSeconds]);

  // Esc cancels.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !creating) onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, creating]);

  const seek = useCallback((t: number) => {
    const v = videoRef.current;
    if (v && Number.isFinite(t)) v.currentTime = clamp(t, 0, duration || t);
  }, [duration]);

  const pxDeltaToTime = (dx: number) => {
    const el = trackRef.current;
    if (!el || duration <= 0) return 0;
    return (dx / el.getBoundingClientRect().width) * duration;
  };

  const onHandleDown = (kind: 'start' | 'end' | 'region') => (e: React.PointerEvent) => {
    if (!edit) return;
    e.preventDefault();
    e.stopPropagation();
    (e.target as Element).setPointerCapture(e.pointerId);
    dragRef.current = { kind, startX: e.clientX, s: start, e: end };
  };

  const onHandleMove = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    const dt = pxDeltaToTime(e.clientX - d.startX);
    if (d.kind === 'start') {
      const s = clamp(d.s + dt, Math.max(0, d.e - MAX_LEN), d.e - MIN_LEN);
      setStart(s);
      seek(s);
    } else if (d.kind === 'end') {
      const en = clamp(d.e + dt, d.s + MIN_LEN, Math.min(duration, d.s + MAX_LEN));
      setEnd(en);
      seek(en);
    } else {
      const len = d.e - d.s;
      const s = clamp(d.s + dt, 0, duration - len);
      setStart(s);
      setEnd(s + len);
      seek(s);
    }
  };

  const onHandleUp = (e: React.PointerEvent) => {
    if (dragRef.current) {
      (e.target as Element).releasePointerCapture?.(e.pointerId);
      dragRef.current = null;
    }
  };

  // Click the track (not a handle) to scrub the preview.
  const onTrackClick = (e: React.MouseEvent) => {
    const el = trackRef.current;
    if (!el || duration <= 0) return;
    const r = el.getBoundingClientRect();
    seek(((e.clientX - r.left) / r.width) * duration);
  };

  // Preview just the selection: jump to start, play, pause at end.
  const previewSelection = () => {
    const v = videoRef.current;
    if (!v) return;
    v.currentTime = start;
    void v.play();
    const stop = () => {
      if (v.currentTime >= end) {
        v.pause();
        v.removeEventListener('timeupdate', stop);
      }
    };
    v.addEventListener('timeupdate', stop);
  };

  const create = async () => {
    if (!edit || creating || !title.trim()) return; // title is required
    setCreating(true);
    try {
      const result = await invoke<{ id: string; edit_url: string }>('finalize_clip', {
        rawMediaId: edit.raw_media_id,
        startSeconds: start,
        durationSeconds: end - start,
        title: title.trim(),
      });
      onClose();
      // You already previewed it while trimming — so go straight to a clean
      // "share your clip" card (no player), just the actions.
      openClipModal(
        `https://clips.twitch.tv/${result.id}`,
        { id: result.id, broadcaster_name: channelName, title: title.trim() || 'Your clip' },
        { created: true, editUrl: result.edit_url, shareOnly: true },
      );
    } catch (e) {
      Logger.error('[ClipEditor] finalize_clip failed:', e);
      addToast(`Couldn't create the clip: ${String(e).replace(/^Error:\s*/, '').slice(0, 140)}`, 'error');
      setCreating(false);
    }
  };

  const len = end - start;
  const pct = (t: number) => (duration > 0 ? (t / duration) * 100 : 0);

  return createPortal(
    <motion.div
      className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-sm"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.15 }}
      onClick={() => !creating && onClose()}
    >
      <motion.div
        className="glass-panel relative w-full max-w-5xl overflow-hidden"
        initial={{ scale: 0.96, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.96, opacity: 0 }}
        transition={{ duration: 0.15, ease: 'easeOut' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 px-4 py-3">
          <Clapperboard size={18} style={{ color: '#9146FF' }} className="flex-shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold text-textPrimary">Trim clip</div>
            <div className="truncate text-xs text-textSecondary">{channelName}</div>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={creating}
            aria-label="Cancel"
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-md text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary disabled:opacity-50"
          >
            <X size={18} />
          </button>
        </div>

        <div className="relative w-full bg-black" style={{ aspectRatio: '16 / 9' }}>
          {edit ? (
            <video
              ref={videoRef}
              src={edit.preview_url}
              controls
              className="h-full w-full"
              onTimeUpdate={(e) => setCurrentTime(e.currentTarget.currentTime)}
            />
          ) : loadError ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-6 text-center">
              <p className="text-sm text-textSecondary">Couldn&apos;t load the footage to trim.</p>
              <p className="max-w-md break-words text-xs text-textSecondary opacity-70">{loadError}</p>
              <button
                type="button"
                onClick={onClose}
                className="rounded-md bg-white/[0.06] px-3 py-1.5 text-xs text-textPrimary transition-colors hover:bg-white/[0.1]"
              >
                Close
              </button>
            </div>
          ) : (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2">
              <Loader2 size={28} className="animate-spin text-textSecondary" />
              <p className="text-xs text-textSecondary">Preparing footage…</p>
            </div>
          )}
        </div>

        {edit && (
          <div className="px-4 pb-3 pt-3">
            {/* Timeline: the full ~90s window. Unselected regions dim so the
                selection pops; the white marker is the playhead; drag a handle to
                set in/out, or drag the body to move the whole selection. */}
            <div
              ref={trackRef}
              onClick={onTrackClick}
              className="relative h-14 w-full cursor-pointer select-none overflow-hidden rounded-lg border border-white/5 bg-black/40"
            >
              {/* glass-exempt: the trim veils and the selected span are marks on the timeline, not surfaces. */}
              <div className="absolute inset-y-0 left-0 bg-black/45" style={{ width: `${pct(start)}%` }} />
              <div
                className="absolute inset-y-0 right-0 bg-black/45"
                style={{ width: `${pct(duration - end)}%` }}
              />
              <div
                // glass-exempt: the selected span is a mark on the timeline, not a surface.
                onPointerDown={onHandleDown('region')}
                onPointerMove={onHandleMove}
                onPointerUp={onHandleUp}
                onClick={(e) => e.stopPropagation()}
                className="absolute inset-y-0 cursor-grab active:cursor-grabbing"
                style={{
                  left: `${pct(start)}%`,
                  width: `${pct(len)}%`,
                  background: 'rgba(145,70,255,0.22)',
                  borderTop: '2px solid #9146FF',
                  borderBottom: '2px solid #9146FF',
                }}
              />
              <div
                className="pointer-events-none absolute inset-y-0 w-0.5 bg-white/90"
                style={{ left: `${pct(currentTime)}%` }}
              />
              {(['start', 'end'] as const).map((h) => (
                <div
                  key={h}
                  onPointerDown={onHandleDown(h)}
                  onPointerMove={onHandleMove}
                  onPointerUp={onHandleUp}
                  onClick={(e) => e.stopPropagation()}
                  className="absolute inset-y-0 flex w-5 -translate-x-1/2 cursor-ew-resize items-center justify-center"
                  style={{ left: `${pct(h === 'start' ? start : end)}%` }}
                >
                  <div
                    className="flex h-full w-2.5 items-center justify-center gap-[2px] rounded"
                    style={{ background: '#9146FF' }}
                  >
                    <span className="h-4 w-px bg-white/70" />
                    <span className="h-4 w-px bg-white/70" />
                  </div>
                </div>
              ))}
            </div>

            <div className="mt-1 flex justify-between text-[10px] tabular-nums text-textSecondary">
              <span>0:00</span>
              <span>{fmt(duration)}</span>
            </div>

            <div className="mt-2 flex items-center justify-between text-xs text-textSecondary">
              <button
                type="button"
                onClick={previewSelection}
                className="glass-button inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-textSecondary transition-all hover:text-textPrimary"
              >
                <Play size={13} fill="currentColor" /> Preview selection
              </button>
              <div className="tabular-nums">
                {fmt(start)} – {fmt(end)} ·{' '}
                <span
                  className={
                    len < MIN_LEN || len > MAX_LEN ? 'text-red-400' : 'font-semibold text-textPrimary'
                  }
                >
                  {len.toFixed(1)}s
                </span>
              </div>
            </div>

            <div className="mt-3 border-t border-white/[0.06] pt-3">
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="Clip title (required)"
                  maxLength={100}
                  className="min-w-0 flex-1 rounded-lg border border-white/[0.06] bg-white/[0.05] px-3 py-2 text-sm text-textPrimary transition-colors placeholder:text-textSecondary focus:border-[rgba(145,70,255,0.55)] focus:bg-white/[0.08] focus:outline-none"
                />
                <button
                  type="button"
                  onClick={onClose}
                  disabled={creating}
                  className="glass-button rounded-lg px-3 py-2 text-sm font-medium text-textSecondary transition-all hover:text-textPrimary disabled:opacity-50"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={create}
                  disabled={creating || len < MIN_LEN || len > MAX_LEN || !title.trim()}
                  className="glass-button inline-flex items-center gap-1.5 rounded-lg px-4 py-2 text-sm font-semibold text-[#bf94ff] transition-all hover:text-[#d4b3ff] disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {creating ? <Loader2 size={15} className="animate-spin" /> : <Clapperboard size={15} />}
                  {creating ? 'Creating…' : 'Create clip'}
                </button>
              </div>
              {!title.trim() && (
                <div className="mt-1.5 text-[11px] text-textSecondary">
                  Add a title to create your clip.
                </div>
              )}
            </div>
          </div>
        )}
      </motion.div>
    </motion.div>,
    document.body,
  );
}
