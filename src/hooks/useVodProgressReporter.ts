import { useEffect, useRef, type RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';

/** Which VOD the attached media element is playing, plus the descriptive
 *  fields Rust keeps so a future "continue watching" row needs no lookup. */
export interface VodProgressTarget {
  videoId: string;
  channelLogin?: string;
  title?: string;
  thumbnailUrl?: string;
}

/** Steady-state checkpoint cadence. `timeupdate` fires ~4 Hz; one IPC call
 *  per five seconds is the whole cost of remembering a position. */
const CHECKPOINT_MS = 5000;

/**
 * Report the playhead of a VOD to Rust, which owns the position store and the
 * resume policy. The media element is the only place the playhead exists, so
 * the WebView samples it here and nowhere else: throttled on `timeupdate`,
 * and immediately on pause, seek, end and teardown so the last position is
 * never more than a checkpoint stale.
 *
 * Passing `null` (live, clip, idle) attaches nothing.
 */
export function useVodProgressReporter(
  videoRef: RefObject<HTMLVideoElement | null>,
  target: VodProgressTarget | null,
): void {
  const videoId = target?.videoId ?? null;
  const channelLogin = target?.channelLogin;
  const title = target?.title;
  const thumbnailUrl = target?.thumbnailUrl;
  const lastSentRef = useRef(0);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !videoId) return;
    let disposed = false;
    lastSentRef.current = 0;

    const send = (force: boolean) => {
      const v = videoRef.current;
      if (!v || disposed) return;
      const position = v.currentTime;
      if (!Number.isFinite(position) || position < 0) return;
      const now = Date.now();
      if (!force && now - lastSentRef.current < CHECKPOINT_MS) return;
      lastSentRef.current = now;
      const duration = Number.isFinite(v.duration) && v.duration > 0 ? v.duration : 0;
      invoke('report_vod_position', {
        videoId,
        positionSecs: position,
        durationSecs: duration,
        channelLogin,
        title,
        thumbnailUrl,
      }).catch((e) => Logger.debug('[VodProgress] report failed:', e));
    };

    const onTimeUpdate = () => {
      if (!video.paused && !video.seeking) send(false);
    };
    const onPause = () => send(true);
    const onSeeked = () => send(true);
    const onEnded = () => send(true);

    video.addEventListener('timeupdate', onTimeUpdate);
    video.addEventListener('pause', onPause);
    video.addEventListener('seeked', onSeeked);
    video.addEventListener('ended', onEnded);

    return () => {
      // Final checkpoint before the listeners go: a stop, a channel switch or
      // an unmount all land the position the viewer actually left at.
      send(true);
      disposed = true;
      video.removeEventListener('timeupdate', onTimeUpdate);
      video.removeEventListener('pause', onPause);
      video.removeEventListener('seeked', onSeeked);
      video.removeEventListener('ended', onEnded);
    };
  }, [videoRef, videoId, channelLogin, title, thumbnailUrl]);
}
