import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { motion, AnimatePresence } from 'framer-motion';
import { X, ExternalLink, Loader2 } from 'lucide-react';
import type Hls from 'hls.js';
import { useAppStore } from '../../stores/AppStore';
import type { MediaInfo } from '../../stores/AppStore';
import type { VodStartInfo } from '../../types';
import { useVodProgressReporter } from '../../hooks/useVodProgressReporter';
import { Logger } from '../../utils/logger';

// A centered overlay player for a Twitch VOD posted in chat — the in-popout
// counterpart to ClipModal, so a streamer in chat-only mode (main app closed) can
// watch a VOD right in the chat window instead of spinning the main app up.
//
// Unlike a clip (a direct signed MP4), a VOD is HLS and needs the shared local
// relay: `start_stream` resolves the VOD and returns a localhost m3u8 we feed to
// hls.js. The caller only opens this when main is CLOSED, so the single shared
// relay is free (no live stream to clobber); closing the modal stops the relay.

interface StreamStartResult {
  url: string;
  quality: string;
  available?: string[];
  vod?: VodStartInfo;
}

export default function VodModal() {
  const vodModal = useAppStore((s) => s.vodModal);
  const close = useAppStore((s) => s.closeVodModal);
  const quality = useAppStore((s) => s.settings?.quality) ?? 'best';

  return (
    <AnimatePresence>
      {vodModal && (
        <VodModalInner
          key={vodModal.url}
          url={vodModal.url}
          info={vodModal.info}
          quality={quality}
          onClose={close}
        />
      )}
    </AnimatePresence>
  );
}

function VodModalInner({
  url,
  info,
  quality,
  onClose,
}: {
  url: string;
  info: MediaInfo;
  quality: string;
  onClose: () => void;
}) {
  const [src, setSrc] = useState<string | null>(null);
  const [vod, setVod] = useState<VodStartInfo | null>(null);
  const [error, setError] = useState(false);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  // Read by the hls.js construction effect (keyed on `src`) without making it
  // re-run: both land in the same start_stream response.
  const vodRef = useRef<VodStartInfo | null>(null);

  useVodProgressReporter(
    videoRef,
    vod
      ? {
          videoId: vod.video_id,
          channelLogin: vod.channel_login ?? info.user_login,
          title: vod.title ?? info.title,
          thumbnailUrl: vod.thumbnail_url ?? info.thumbnail_url,
        }
      : null,
  );

  // Resolve the VOD to a localhost HLS URL via the shared relay. Stop the relay on
  // close — in chat-only mode it was ours (main is closed, no other consumer).
  // A new url or quality is a new playback: clear the old source during
  // render so a stale frame never paints under the new request.
  const playbackKey = `${url}|${quality}`;
  const [seenPlaybackKey, setSeenPlaybackKey] = useState(playbackKey);
  if (playbackKey !== seenPlaybackKey) {
    setSeenPlaybackKey(playbackKey);
    setSrc(null);
    setVod(null);
    setError(false);
  }

  useEffect(() => {
    let cancelled = false;
    invoke<StreamStartResult>('start_stream', { url, quality })
      .then((r) => {
        if (cancelled) return;
        vodRef.current = r.vod ?? null;
        setVod(r.vod ?? null);
        setSrc(r.url);
      })
      .catch((e) => {
        if (cancelled) return;
        Logger.error('[VodModal] start_stream failed:', e);
        setError(true);
      });
    return () => {
      cancelled = true;
      void invoke('stop_stream').catch(() => {});
    };
  }, [url, quality]);

  // Attach hls.js once we have the localhost URL. WebView2 (Chromium) has no native
  // HLS, so hls.js is required; the dynamic import keeps it out of the popout's
  // baseline bundle until a VOD actually plays.
  useEffect(() => {
    if (!src) return;
    let cancelled = false;
    let hls: Hls | null = null;
    void (async () => {
      const video = videoRef.current;
      if (!video) return;
      try {
        const { default: HlsCtor } = await import('hls.js');
        if (cancelled || !videoRef.current) return;
        if (HlsCtor.isSupported()) {
          // A VOD, never live: a still-recording VOD is a growing EVENT
          // playlist hls.js would otherwise pin to its tail. Start at the
          // stored resume position when Rust supplied one.
          hls = new HlsCtor({
            liveDurationInfinity: false,
            startPosition: vodRef.current?.start_position_secs ?? -1,
          });
          hls.loadSource(src);
          hls.attachMedia(video);
          hls.on(HlsCtor.Events.ERROR, (_evt, data) => {
            if (data?.fatal) {
              Logger.error('[VodModal] hls fatal error:', data);
              setError(true);
            }
          });
        } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
          video.src = src;
        } else {
          setError(true);
        }
      } catch (e) {
        Logger.error('[VodModal] hls load failed:', e);
        setError(true);
      }
    })();
    return () => {
      cancelled = true;
      if (hls) {
        try {
          hls.destroy();
        } catch {
          /* ignore */
        }
      }
      const v = videoRef.current;
      if (v) {
        v.pause();
        v.removeAttribute('src');
        v.load();
      }
    };
  }, [src]);

  // Esc closes — counterpart to the backdrop click and the X.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const openExternal = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-shell');
      await open(url);
    } catch (err) {
      Logger.error('[VodModal] external open failed:', err);
    }
  };

  const channel = info.broadcaster_name || info.user_name;

  return createPortal(
    <motion.div
      className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-sm"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.15 }}
      onClick={onClose}
    >
      <motion.div
        className="glass-panel relative w-full max-w-7xl overflow-hidden"
        initial={{ scale: 0.96, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.96, opacity: 0 }}
        transition={{ duration: 0.15, ease: 'easeOut' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-2.5">
          <div className="min-w-0 flex-1">
            {info.title && (
              <div className="truncate text-sm font-medium text-textPrimary">{info.title}</div>
            )}
            <div className="truncate text-xs text-textSecondary">
              <span style={{ color: '#9146FF' }} className="font-semibold">
                Twitch
              </span>
              {channel ? ` · ${channel}` : ''}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close video"
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-md text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
          >
            <X size={18} />
          </button>
        </div>

        <div className="relative w-full bg-black" style={{ aspectRatio: '16 / 9' }}>
          {error ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-6 text-center">
              <p className="text-sm text-textSecondary">Couldn&apos;t load this video.</p>
              <button
                type="button"
                onClick={openExternal}
                className="inline-flex items-center gap-1.5 rounded-md bg-white/[0.06] px-3 py-1.5 text-xs text-textPrimary transition-colors hover:bg-white/[0.1]"
              >
                <ExternalLink size={13} /> Open in browser
              </button>
            </div>
          ) : (
            <>
              <video ref={videoRef} autoPlay controls className="h-full w-full" onError={() => setError(true)} />
              {!src && (
                <div className="absolute inset-0 flex items-center justify-center">
                  <Loader2 size={28} className="animate-spin text-textSecondary" />
                </div>
              )}
            </>
          )}
        </div>
      </motion.div>
    </motion.div>,
    document.body,
  );
}
