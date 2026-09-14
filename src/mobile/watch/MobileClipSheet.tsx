// Playing a Twitch clip on the phone, without losing the live stream.
//
// A clip link in chat used to be a DEAD TAP on mobile. LinkPreviewCard routes a
// clip to `openClipModal`, which flips a store field - but <ClipModal /> is
// mounted only in the desktop shell (App.tsx), and MobileApp never imported it.
// The state changed and nothing rendered.
//
// This is the mobile consumer of that same store field.
//
// It plays the clip in ITS OWN <video>, deliberately NOT through the mobile HLS
// engine. Two reasons, both load-bearing:
//   1. `resolve_clip_media` returns a signed direct MP4, and useMobileHlsEngine
//      unconditionally does `hls.loadSource(streamUrl)` with no MP4 branch -
//      handing it an MP4 simply fails.
//   2. The engine's own header documents that there is exactly one player
//      instance and that its module state would have to move into the hook if a
//      second surface appeared. A private <video> here avoids forcing that.
//
// The Rust side was already built for this shape: resolve_clip_media's docstring
// says it resolves "WITHOUT touching any global live-stream state ... so the
// main stream/chat keeps running underneath and the user lands back exactly
// where they were when the modal closes."
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { MobileSheet } from '../ui/MobileSheet';
import { duckLiveAudio } from '../player/liveVideo';
import { openExternal } from '../../utils/openExternal';
import { Logger } from '../../utils/logger';
import PenroseMarch from '../../components/PenroseMarch';

interface ClipResolveResult {
  url: string;
  quality: string;
  available: string[];
}

// Inner body, mounted with `key={url}`. Keying rather than syncing state in an
// effect is what keeps this free of the cascading-render pattern the repo lints
// against: a different clip is a different component instance, so `src` and
// `failed` reset by construction instead of being cleared on the way in.
const ClipBody: React.FC<{ url: string; title?: string; onClose: () => void }> = ({
  url,
  title,
  onClose,
}) => {
  const [src, setSrc] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<ClipResolveResult>('resolve_clip_media', { url, quality: 'best' })
      .then((r) => {
        if (!cancelled) setSrc(r.url);
      })
      .catch((e) => {
        if (cancelled) return;
        // Hand it to the OS rather than leaving a sheet spinning forever: the
        // phone can still play this in the Twitch app or a browser, which is
        // strictly better than a spinner that never resolves.
        Logger.error('[MobileClipSheet] resolve failed, handing to the OS:', e);
        void openExternal(url);
        onClose();
      });
    return () => {
      cancelled = true;
    };
  }, [url, onClose]);

  // Duck the live stream for exactly as long as this clip is mounted. One phone,
  // one speaker: otherwise the live audio and the clip audio both play. Mute
  // rather than pause, so the live stream stays at the live edge and does not
  // have to catch up when the sheet closes.
  useEffect(() => duckLiveAudio(), []);

  // Release the media element when it detaches. A detached <video> can keep
  // decoding, buffering, and even playing audio until GC reclaims it; pausing,
  // clearing the src and calling load() frees it immediately. Via a ref callback
  // (React passes null on detach) so we always release the exact node that is
  // going away. Same teardown the desktop ClipModal relies on.
  const attachVideo = useCallback((el: HTMLVideoElement | null) => {
    if (el === null && videoRef.current) {
      const prev = videoRef.current;
      prev.pause();
      prev.removeAttribute('src');
      prev.load();
    }
    videoRef.current = el;
  }, []);

  return (
    <MobileSheet open onClose={onClose} title={title ?? 'Clip'} maxHeightFraction={0.6}>
      <div className="px-3 pb-3">
        <div className="relative w-full aspect-video bg-black rounded-lg overflow-hidden">
          {src ? (
            <video
              ref={attachVideo}
              src={src}
              className="w-full h-full object-contain"
              // `controls` because a clip is seekable, VOD-shaped media, unlike
              // the live player's custom tap overlay. Native controls give scrub
              // for free and cost nothing to maintain.
              controls
              playsInline
              autoPlay
            />
          ) : (
            <div className="absolute inset-0 grid place-items-center">
              <PenroseMarch />
            </div>
          )}
        </div>
      </div>
    </MobileSheet>
  );
};

export const MobileClipSheet: React.FC = () => {
  const clipModal = useAppStore((s) => s.clipModal);
  const closeClipModal = useAppStore((s) => s.closeClipModal);

  if (!clipModal) return null;
  return (
    <ClipBody
      key={clipModal.url}
      url={clipModal.url}
      title={clipModal.info?.title ?? undefined}
      onClose={closeClipModal}
    />
  );
};
