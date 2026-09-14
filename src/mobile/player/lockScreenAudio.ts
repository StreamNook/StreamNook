// Lock-screen audio, and the media controls that go with it.
//
// THE BUG THIS FIXES: locking the phone killed playback outright. Two causes,
// both native:
//   1. `WryActivity.onPause()` calls `mWebView.onPause()`, which suspends the
//      WebView's media along with its rendering.
//   2. Even awake, a backgrounded process with no foreground service gets
//      frozen by the OS.
// MediaPlaybackService fixes both, and MainActivity undoes the WebView pause
// while it runs. This module is the web half: it tells the service what is
// playing, and applies the transport commands that come back.
//
// The <video> element is the SOURCE OF TRUTH for play state. The lock screen
// button sends a command, we act on the element, and the element's own play /
// pause events report the result back. State never flows the other way, so the
// notification can never show "paused" over a stream that is still playing.
import { isInPip, mediaSessionStart, mediaSessionStop } from '../nativeBridge';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

export interface NowPlaying {
  /** Stream title, or the channel name when there is no title. */
  title: string;
  /** Channel display name. */
  artist: string;
  /** Channel avatar for the notification thumbnail. Empty is fine. */
  artUrl: string;
}

let current: NowPlaying | null = null;
let attached: HTMLVideoElement | null = null;
let detach: (() => void) | null = null;
// Whether we are currently on the audio-only rendition, and what to put back.
//
// Two variables, not one. Using the restore target as the "are we downshifted"
// flag looks tidier and is broken: the target can legitimately be null, which
// makes the guard useless and lets both triggers fire a re-resolve.
let downshifted = false;
let restoreQuality: string | null = null;

function pushState(playing: boolean): void {
  if (!current) return;
  mediaSessionStart(current.title, current.artist, current.artUrl, playing);
}

/**
 * Bind a video element to the lock-screen session.
 *
 * Returns a teardown. Safe to call repeatedly; rebinding replaces the previous
 * binding rather than stacking listeners.
 */
export function attachLockScreenAudio(video: HTMLVideoElement, info: NowPlaying): () => void {
  // Rebinding the SAME element with the same metadata is a no-op beyond the
  // state push. MobilePlayer re-renders often and must not accumulate listeners.
  if (attached === video && current && current.title === info.title) {
    pushState(!video.paused);
    return () => releaseLockScreenAudio();
  }

  detach?.();
  attached = video;
  current = info;

  const onPlay = () => pushState(true);
  const onPause = () => pushState(false);
  // `ended` on a live stream means the broadcast stopped. Keeping a media
  // notification for a stream that is over is just litter.
  const onEnded = () => releaseLockScreenAudio();

  video.addEventListener('play', onPlay);
  video.addEventListener('pause', onPause);
  video.addEventListener('ended', onEnded);

  // Transport commands from the lock screen, the shade, a headset button, or an
  // audio-focus change. MainActivity raises this from the native session.
  const onCommand = (e: Event) => {
    const cmd = (e as CustomEvent<string>).detail;
    const v = attached;
    if (!v) return;
    if (cmd === 'play') {
      // A rejected play() is normal (autoplay policy, focus not granted yet) and
      // must not throw into the event handler.
      void v.play().catch((err) => Logger.warn('[lockScreenAudio] play rejected', err));
    } else if (cmd === 'pause') {
      v.pause();
    } else if (cmd === 'stop') {
      v.pause();
      releaseLockScreenAudio();
    }
  };
  window.addEventListener('sn:media-cmd', onCommand);

  // Whenever the player loses its surface, drop to an audio-only rendition;
  // restore when it comes back.
  //
  // This is NOT a battery optimisation, it is what makes background audio work
  // at all. Losing the surface - screen off, or backgrounding without PiP -
  // leaves Chromium holding a WebContents with a video track and nowhere to
  // render it, and it tears the media pipeline down. Measured: the app's
  // AudioPlaybackConfiguration disappears from `dumpsys audio` entirely rather
  // than moving to a paused state (`viewVisibility=8`, then
  // `NO_SURFACE; reason: destroySurface`). Audio-only media has nothing to
  // render, so it survives. Xtra reaches the same place by disabling the video
  // TRACK; we resolve to a single muxed variant, so swapping RENDITIONS is our
  // equivalent.
  //
  // The reload it costs is invisible: nobody is looking at the video.
  //
  // PiP is the one case that must NOT downshift. A PiP window is a real surface
  // showing real video, so dropping the video track there would blank it.
  const downshift = () => {
    const v = attached;
    if (!v || v.paused) return;
    // `downshifted` is a SEPARATE boolean from the restore target on purpose.
    // An earlier version used the target itself as the guard, and it did not
    // hold: `activeQuality` can be null, so the guard stayed falsy, both
    // triggers ran, and locking the phone fired TWO full re-resolves 0.4s apart
    // (two LLOrigin activations against different playlist servers, three audio
    // sessions). That doubled gap is what read as a glitchy freeze on lock.
    if (downshifted) return;
    if (isInPip() === true) return;
    downshifted = true;
    // 'best' is the honest fallback: it is what the resolver defaults to, so
    // restoring it returns the viewer to auto rather than stranding them on
    // audio.
    restoreQuality = useAppStore.getState().activeQuality || 'best';
    void useAppStore.getState().applyTransientQuality('audio_only');
  };
  const restore = () => {
    if (!downshifted) return;
    downshifted = false;
    const q = restoreQuality || 'best';
    restoreQuality = null;
    void useAppStore.getState().applyTransientQuality(q);
  };

  // Two independent triggers, because they are genuinely different events and
  // neither implies the other:
  //   - screen off/on, which can happen while the app is in the foreground.
  //   - the app being hidden, which covers backgrounding without PiP (the
  //     "minimize to audio" path) and does not fire for a screen lock in PiP.
  const onVisibility = () => (document.hidden ? downshift() : restore());
  window.addEventListener('sn:screen-off', downshift);
  window.addEventListener('sn:screen-on', restore);
  document.addEventListener('visibilitychange', onVisibility);

  detach = () => {
    video.removeEventListener('play', onPlay);
    video.removeEventListener('pause', onPause);
    video.removeEventListener('ended', onEnded);
    window.removeEventListener('sn:media-cmd', onCommand);
    window.removeEventListener('sn:screen-off', downshift);
    window.removeEventListener('sn:screen-on', restore);
    document.removeEventListener('visibilitychange', onVisibility);
  };

  pushState(!video.paused);
  return () => releaseLockScreenAudio();
}

/**
 * Drop the session and let the foreground service stop.
 *
 * Worth doing promptly rather than leaving it to teardown: while the service
 * runs the WebView is deliberately kept awake in the background, which is
 * exactly the battery cost this whole feature exists to make WORTH paying. It
 * should not outlive the audio it is protecting.
 */
export function releaseLockScreenAudio(): void {
  detach?.();
  detach = null;
  attached = null;
  current = null;
  // Reset the downshift state with everything else. Leaving it set would make
  // the NEXT stream's first lock a no-op, because the guard would still think
  // we were already on the audio rendition.
  downshifted = false;
  restoreQuality = null;
  mediaSessionStop();
}
