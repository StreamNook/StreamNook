// Standalone hls.js engine for the mobile player. Deliberately NOT extracted
// from VideoPlayer.tsx: the desktop createPlayer interleaves Plyr construction
// and window orchestration that took months to settle; this is the lean subset
// a phone needs. The latency handling below is a faithful copy of the desktop's
// rather than a shared module, for the same reason.
//
// Three pieces work together and none of them does anything alone.
//
// The PROBE decides which path this stream is on. Low latency is a per-stream
// fact, not a preference: the setting arms the backend, and the backend only
// serves the low-latency origin when the channel actually supports it. Asking
// is the only way to know, and it has to happen before the player is built,
// because hls.js can only be told at construction.
//
// The BUFFER GATE decides where playback starts. Without it the cold-start
// buffer, not the configured cushion, sets how far behind live you sit: the
// buffer accumulates behind an advancing edge, so playback begins wherever that
// pile started and stays there. Snapping to the freshest buffered moment minus
// the cushion is what makes the setting mean anything.
//
// The GOVERNOR keeps it there. hls.js's own catch-up is switched off (its rate
// steps are coarse enough to hear), so without something holding the playhead
// it drifts further behind across a session. It only ever nudges playbackRate,
// never seeks; seeking a live stream toward the edge mid-playback freezes it.
//
// Before these landed the phone had none of the three, which is why the Low
// Latency setting appeared to do nothing at all: the backend was serving the
// right thing and the player was ignoring it.
import { useEffect, useRef, useState } from 'react';
import Hls from 'hls.js';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { setActiveVideo } from '../../utils/activeVideo';
import { startLatencyGovernor } from '../../utils/liveLatencyGovernor';
import { LL_DISPLAY_CALIBRATION, LL_TARGET_DEFAULT } from '../../utils/latency';
import { Logger } from '../../utils/logger';

// The earning gate.
//
// The Rust watch heartbeat credits a minute-watched (drops progress + channel
// points) only while the player says it is playing, and it samples that flag
// INSTANTANEOUSLY once every 60 seconds. A two-second window where the flag
// reads false can therefore cost a whole minute, which is why this mirrors the
// desktop VideoPlayer's handlers rather than reporting every state the media
// element passes through.
//
// Desktop rules, ported from VideoPlayer.tsx:
//   - `playing` reports true at once and cancels any pending paused report.
//   - `pause` does NOT report false. It arms a grace timer, and only a pause
//     that outlives the timer reports false.
//   - There is no `waiting` handler. A buffer underrun is not a pause. Phones
//     underrun constantly, and reporting false on each one is the whole reason
//     earning was spotty here while the desktop was flawless.
//   - Teardown cancels the pending report and reports nothing. Desktop does
//     exactly this in an unmount effect, for the reason its comment gives: a
//     report that lands after the player is gone gates the NEXT stream off.
//
// Module scope rather than refs because there is exactly one player: one
// useMobileHlsEngine call site, in one MobilePlayer, in one WatchScreen. If a
// second player surface is ever added, this state has to move into the hook.
const PAUSE_REPORT_GRACE_MS = 6000;

// A wedged link must not block every later report for the life of the process,
// which would recreate the exact "video plays, nothing earns" failure this
// whole gate exists to prevent.
const REPORT_TIMEOUT_MS = 2000;

// How long a pause must last before we stop pulling segments.
//
// hls.js keeps filling the buffer while the element is paused, so a paused
// stream downloads at full bitrate indefinitely. Measured on device: the buffer
// grew by exactly 2.00s per 2s of wall clock until it reached
// `liveMaxLatencyDuration`, at which point hls seeked ~56s to the live edge and
// discarded the entire buffer - then did it again, every ~55 seconds, for as
// long as the stream sat paused. A paused phone downloads a live stream
// continuously and throws all of it away.
//
// The grace window exists so a quick pause/resume, or a transient pause from
// audio focus, does not tear the buffer down for nothing. Anything longer is a
// real pause and worth stopping for.
const STOP_LOAD_AFTER_PAUSE_MS = 2000;

let pausedReportTimer: number | null = null;

// `report_player_playing` is an async command, so Tauri spawns each call as its
// own task and two calls have no ordering relationship. Chaining makes the last
// call made the last call applied. Insurance, not a fix for a known trigger.
let reportChain: Promise<unknown> = Promise.resolve();

function sendPlaying(playing: boolean): void {
  Logger.debug(`[MobilePlayer] earning gate -> ${playing ? 'playing' : 'paused'}`);
  reportChain = reportChain.then(() =>
    Promise.race([
      invoke('report_player_playing', { playing }).catch((err) => {
        Logger.warn('[MobilePlayer] report_player_playing failed:', err);
      }),
      new Promise((resolve) => window.setTimeout(resolve, REPORT_TIMEOUT_MS)),
    ]),
  );
}

function cancelPausedReport(): void {
  if (pausedReportTimer !== null) {
    window.clearTimeout(pausedReportTimer);
    pausedReportTimer = null;
  }
}

function reportPlaying(): void {
  cancelPausedReport();
  sendPlaying(true);
}

function reportPausedSoon(reason: string): void {
  cancelPausedReport();
  pausedReportTimer = window.setTimeout(() => {
    pausedReportTimer = null;
    Logger.debug(`[MobilePlayer] pause outlived the grace window (${reason})`);
    sendPlaying(false);
  }, PAUSE_REPORT_GRACE_MS);
}

export type MobilePlayerState = 'idle' | 'loading' | 'playing' | 'stalled' | 'error';

export function useMobileHlsEngine(videoRef: React.RefObject<HTMLVideoElement | null>) {
  const streamUrl = useAppStore((s) => s.streamUrl);
  const playerSettings = useAppStore((s) => s.settings.video_player);
  const [state, setState] = useState<MobilePlayerState>('idle');
  const hlsRef = useRef<Hls | null>(null);
  const seqRef = useRef(0);

  // Read when the player is built, so moving a slider does not tear down and
  // rebuild a running stream. The governor re-reads its target every tick, so
  // the live-edge gap still applies mid-stream without a rebuild.
  const settingsRef = useRef(playerSettings);
  useEffect(() => {
    settingsRef.current = playerSettings;
  }, [playerSettings]);

  useEffect(() => {
    const video = videoRef.current;
    const seq = ++seqRef.current;
    let cancelled = false;
    let stopGovernor: (() => void) | null = null;
    let detachVideo: (() => void) | null = null;
    let gateTimer: number | null = null;
    let stopLoadTimer: number | null = null;
    let loadStopped = false;

    const destroy = () => {
      if (gateTimer !== null) {
        window.clearTimeout(gateTimer);
        gateTimer = null;
      }
      if (stopLoadTimer !== null) {
        window.clearTimeout(stopLoadTimer);
        stopLoadTimer = null;
      }
      loadStopped = false;
      if (stopGovernor) {
        stopGovernor();
        stopGovernor = null;
      }
      if (hlsRef.current) {
        try {
          hlsRef.current.destroy();
        } catch {
          /* already dead */
        }
        hlsRef.current = null;
      }
    };

    if (!video || !streamUrl || streamUrl === 'offline') {
      destroy();
      setState(streamUrl === 'offline' ? 'error' : 'idle');
      return;
    }

    if (!Hls.isSupported()) {
      // Android System WebView always has MSE; this is a diagnostics guard.
      Logger.error('[MobilePlayer] hls.js unsupported in this webview');
      setState('error');
      return;
    }

    setState('loading');

    void (async () => {
      // Whether the relay is serving the parts-based low-latency origin for
      // THIS stream. Deliberately not read from the setting: turning the
      // setting on for a channel that cannot do it would switch hls.js into
      // low-latency mode against an ordinary playlist, which starves the
      // buffer, and would also hand playhead control to a controller the
      // governor is meant to own.
      let isLowLatencyChannel = false;
      try {
        isLowLatencyChannel = await invoke<boolean>('get_stream_low_latency');
      } catch {
        /* command unavailable or the stream went away */
      }
      // A newer stream (or teardown) owns the element now; building here would
      // leave a zombie player attached to it.
      if (cancelled || seq !== seqRef.current) return;

      const current = settingsRef.current;
      // The viewer's chosen distance behind live. On the parts path the number
      // the overlay shows is calibrated down by a fixed amount, so aim that
      // much higher to land on what they actually asked for.
      const llTargetDisplayed = current?.ll_target_latency ?? LL_TARGET_DEFAULT;
      const llTargetRaw = isLowLatencyChannel
        ? llTargetDisplayed + LL_DISPLAY_CALIBRATION
        : llTargetDisplayed;

      const hls = new Hls({
        enableWorker: true,
        lowLatencyMode: isLowLatencyChannel,
        startFragPrefetch: false,
        backBufferLength: 30,
        maxBufferLength: current?.max_buffer_length || 30,
        maxMaxBufferLength: current?.max_buffer_length || 120,
        maxBufferSize: 60 * 1000 * 1000,
        maxBufferHole: 0.5,
        highBufferWatchdogPeriod: 2,
        nudgeOffset: 0.2,
        nudgeMaxRetry: 3,
        maxFragLookUpTolerance: 0.5,
        liveSyncDuration: llTargetRaw,
        liveMaxLatencyDuration: 60,
        // hls.js's own catch-up stays off on both paths: its rate steps are
        // quantized coarsely enough to be audible. The governor below does it
        // smoothly instead.
        maxLiveSyncPlaybackRate: 1,
        liveDurationInfinity: true,
        manifestLoadingTimeOut: 10000,
        manifestLoadingMaxRetry: 3,
        manifestLoadingRetryDelay: 1000,
        levelLoadingTimeOut: 10000,
        levelLoadingMaxRetry: 4,
        levelLoadingRetryDelay: 1000,
        fragLoadingTimeOut: 20000,
        fragLoadingMaxRetry: 6,
        fragLoadingRetryDelay: 1000,
        startLevel: current?.start_quality ?? -1,
        abrEwmaDefaultEstimate: 1_500_000,
        abrEwmaFastLive: 3.0,
        abrEwmaSlowLive: 9.0,
        abrBandWidthFactor: 0.95,
        abrBandWidthUpFactor: 0.7,
      });
      hlsRef.current = hls;

      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        if (seq !== seqRef.current) return;
        // Only the low-latency path starts here. hls.js positions the playhead
        // itself on that path, so there is nothing to wait for. The ordinary
        // path starts from the buffer gate below.
        if (!isLowLatencyChannel) return;
        video.play().catch((err) => {
          Logger.warn('[MobilePlayer] autoplay rejected:', err);
        });
      });

      // Cold-start buffer gate and live-sync snap.
      //
      // Threshold is 3 and not 4 on purpose: two 2-second segments measure as
      // roughly 3.96s, which just misses 4 and forces a wait for a third
      // segment that has not been produced yet.
      let playStarted = false;
      const GATE_THRESHOLD = isLowLatencyChannel ? 1 : 3;

      hls.on(Hls.Events.FRAG_BUFFERED, () => {
        if (seq !== seqRef.current) return;
        if (playStarted || isLowLatencyChannel) return;

        const buffered = video.buffered;
        if (buffered.length === 0) return;
        const bufStart = buffered.start(0);
        const bufEnd = buffered.end(buffered.length - 1);
        if (bufEnd - bufStart < GATE_THRESHOLD) return;

        playStarted = true;
        // Clamped into what is actually buffered so this can never seek into a
        // hole, and a no-op when the buffer is already inside the cushion, so
        // it never drags anyone CLOSER to live than they asked to be.
        const syncDur = hls.config.liveSyncDuration ?? llTargetRaw;
        const target = Math.max(bufStart, bufEnd - syncDur);
        if (target > video.currentTime + 0.5) video.currentTime = target;

        video.play().catch(() => {
          video.muted = true;
          video.play().catch(() => Logger.warn('[MobilePlayer] muted autoplay also failed'));
        });
      });

      // If the gate never clears (thin delivery on a phone connection), start
      // anyway rather than sit on a black frame. Low latency does not need it;
      // it already started above.
      if (!isLowLatencyChannel) {
        gateTimer = window.setTimeout(() => {
          if (playStarted || seq !== seqRef.current) return;
          playStarted = true;
          Logger.warn('[MobilePlayer] buffer gate timed out, starting anyway');
          video.play().catch(() => {});
        }, 5000);
      }

      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (seq !== seqRef.current) return;
        if (!data.fatal) return;
        Logger.warn('[MobilePlayer] fatal hls error:', data.type, data.details);
        switch (data.type) {
          case Hls.ErrorTypes.NETWORK_ERROR:
            setState('stalled');
            hls.startLoad();
            break;
          case Hls.ErrorTypes.MEDIA_ERROR:
            setState('stalled');
            hls.recoverMediaError();
            break;
          default:
            setState('error');
            destroy();
        }
      });

      // Segment loading follows the play state.
      //
      // Deliberately driven off `play`/`pause` (intent) rather than
      // `playing`/`waiting` (buffer health), so an ordinary rebuffer never stops
      // the loader that is trying to end it.
      const resumeLoad = (why: string) => {
        if (stopLoadTimer !== null) {
          window.clearTimeout(stopLoadTimer);
          stopLoadTimer = null;
        }
        if (!loadStopped) return;
        loadStopped = false;
        try {
          // No start position: resume buffering around the current playhead, so
          // resuming behaves exactly as it does today. Passing -1 here would
          // snap to the live edge and silently skip content on a short pause.
          hls.startLoad();
          Logger.debug(`[MobilePlayer] segment loading resumed (${why})`);
        } catch {
          /* player already torn down */
        }
      };

      const onPlayIntent = () => {
        if (seq !== seqRef.current) return;
        resumeLoad('play');
      };

      const onPlaying = () => {
        if (seq !== seqRef.current) return;
        // Belt and braces: if anything resumed playback without a `play` event
        // reaching us, do not leave the loader stopped under a running element.
        resumeLoad('playing');
        setState('playing');
        reportPlaying();
      };
      const onWaiting = () => {
        if (seq !== seqRef.current) return;
        // Spinner only. The earning gate is deliberately untouched here; see
        // the note above the gate helpers.
        Logger.debug('[MobilePlayer] buffer stall (earning gate unchanged)');
        setState('stalled');
      };
      const onPauseOrEnd = () => {
        if (seq !== seqRef.current) return;
        // A pause that arrives while the page is already hidden cannot be
        // deferred: Android pauses the WebView along with the activity, so the
        // timer would never fire. In picture-in-picture the native layer
        // resumes the WebView on purpose so playback continues, which means a
        // pause there is a real pause and reporting at once is still right. No
        // PiP signal is needed, and no race with it exists.
        // Stop pulling segments once the pause outlives the grace window. See
        // STOP_LOAD_AFTER_PAUSE_MS: without this a paused stream downloads at
        // full bitrate forever and discards the lot every ~55 seconds.
        //
        // Note this is armed even when the page is hidden. Backgrounding does
        // NOT pause playback here (lifecycle.ts leaves the audio alone on
        // purpose), so a pause event while hidden is a genuine pause, not the
        // app going to the background.
        if (stopLoadTimer === null && !loadStopped) {
          stopLoadTimer = window.setTimeout(() => {
            stopLoadTimer = null;
            if (seq !== seqRef.current || !video.paused || loadStopped) return;
            loadStopped = true;
            try {
              hls.stopLoad();
              Logger.debug('[MobilePlayer] paused past the grace window, segment loading stopped');
            } catch {
              /* player already torn down */
            }
          }, STOP_LOAD_AFTER_PAUSE_MS);
        }

        if (document.visibilityState === 'hidden') {
          cancelPausedReport();
          sendPlaying(false);
          return;
        }
        reportPausedSoon('pause/ended');
      };
      const onVisibilityChange = () => {
        if (seq !== seqRef.current) return;
        // The other ordering: pause landed first and armed the timer, then the
        // app went to the background. Flush it while the DOM still runs.
        if (document.visibilityState === 'hidden' && pausedReportTimer !== null) {
          cancelPausedReport();
          Logger.debug('[MobilePlayer] flushing pending paused report on hide');
          sendPlaying(false);
        }
      };
      video.addEventListener('play', onPlayIntent);
      video.addEventListener('playing', onPlaying);
      video.addEventListener('waiting', onWaiting);
      video.addEventListener('pause', onPauseOrEnd);
      video.addEventListener('ended', onPauseOrEnd);
      document.addEventListener('visibilitychange', onVisibilityChange);
      detachVideo = () => {
        video.removeEventListener('play', onPlayIntent);
        video.removeEventListener('playing', onPlaying);
        video.removeEventListener('waiting', onWaiting);
        video.removeEventListener('pause', onPauseOrEnd);
        video.removeEventListener('ended', onPauseOrEnd);
        document.removeEventListener('visibilitychange', onVisibilityChange);
      };

      hls.loadSource(streamUrl);
      hls.attachMedia(video);
      // Register as the active video so app-wide integrations (volume, stats,
      // watch heartbeat consumers) see this element like the desktop player's.
      setActiveVideo(video);

      // Same two profiles the desktop runs. The low-latency one drives the
      // playhead toward a behind-live target and is tuned tightly enough that
      // it needs the buffer floor to stay safe; the ordinary one just mops up
      // drift against the forward buffer.
      stopGovernor = isLowLatencyChannel
        ? startLatencyGovernor(hls, video, {
            label: 'mobile-ll',
            // Read live rather than captured, so moving the gap slider applies
            // without rebuilding the player.
            latencyTarget: () =>
              (settingsRef.current?.ll_target_latency ?? LL_TARGET_DEFAULT) +
              LL_DISPLAY_CALIBRATION,
            getLatency: () =>
              typeof hls.latency === 'number' && hls.latency > 0 ? hls.latency : null,
            gain: 0.12,
            // ceiling and engageSpan are inherited from the shared governor
            // defaults (1.05, buffer-aware engage). Phones hit the unbacked
            // catch-up failure more than desktop, and the buffer-aware engage
            // span is exactly the fix for it, so no mobile override here.
            band: 0.1,
            floor: 0.8,
            slowRate: 0.97,
            tickMs: 500,
            rampStep: 0.01,
            log: Logger.debug,
          })
        : startLatencyGovernor(hls, video, { label: 'mobile', log: Logger.debug });
    })();

    return () => {
      cancelled = true;
      detachVideo?.();
      // Cancel, do not report. This is a port of the desktop unmount effect in
      // VideoPlayer.tsx, whose comment gives the reason: a paused report that
      // lands after the player is gone wrongly gates the NEXT stream's earning
      // off. It also removes the dead window that a restart, a quality switch
      // and an ad pivot used to pay, since all three swap streamUrl while
      // deliberately leaving drops monitoring armed on the same channel. Real
      // teardown needs no help here: stopStream and exitStream both reach
      // stop_drops_monitoring, whose clear_target drops the flag and the target
      // together.
      cancelPausedReport();
      if (seq === seqRef.current) setActiveVideo(null);
      destroy();
    };
    // videoRef identity is stable (a ref); streamUrl drives the lifecycle.
    // Settings are read through a ref on purpose, so changing one does not
    // rebuild a running player.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamUrl]);

  return { state };
}
