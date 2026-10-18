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
import { behindLiveFromEdge, edgeTargetForGap, resolveLiveEdgeGap } from '../../utils/latency';
import { Logger } from '../../utils/logger';
import { recoverFromSequenceMismatch } from '../../utils/hlsSequenceMismatch';
import { createStallReporter, linkCushion, refreshLinkCushion, reportSegment } from './linkAdapt';

// The earning gate (drops progress, channel points, watch time) is Rust's
// (services/earning_gate.rs): the player only says what its video does. A
// rebuffer is not a pause, so `waiting` is never reported.
function reportState(state: 'playing' | 'paused' | 'paused_now' | 'hidden' | 'teardown'): void {
  invoke('report_player_state', { state }).catch((err) => {
    Logger.warn('[MobilePlayer] report_player_state failed:', err);
  });
}

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

export type MobilePlayerState = 'idle' | 'loading' | 'playing' | 'stalled' | 'error';

/** Why playback gave up, so the error line can say it in plain words. */
export type MobilePlayerFailure = 'network' | 'media' | 'stall' | 'timeout' | 'unsupported' | 'other';

// A failure is retried a set number of times and then shown, never looped: the
// low-latency route falls back to whole segments once, then the player is
// rebuilt twice (after 1 s, then 2 s) before the error and its Retry appear.
// hls.js's own recovery used to run forever under an endless loading mark.
const MAX_RELOADS = 2;
// A rebuffer that makes no progress for this long is a dead stream, not a slow
// one. Without it a relay that keeps answering with nothing new held the
// loading mark up indefinitely, since hls.js never calls that fatal.
const STALL_GIVE_UP_MS = 20_000;
// No first frame this long after the player was built: the start failed
// quietly (every manifest and segment timeout is already shorter than this).
const START_GIVE_UP_MS = 30_000;

export function useMobileHlsEngine(videoRef: React.RefObject<HTMLVideoElement | null>) {
  const streamUrl = useAppStore((s) => s.streamUrl);
  const playerSettings = useAppStore((s) => s.settings.video_player);
  const [state, setState] = useState<MobilePlayerState>('idle');
  const [reason, setReason] = useState<MobilePlayerFailure | null>(null);
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
    let cushionTimer: number | null = null;
    let startGiveUpTimer: number | null = null;
    let stallGiveUpTimer: number | null = null;
    let reloadTimer: number | null = null;
    let loadStopped = false;
    // The route the current player was built on, whether it already fell back
    // from low latency, and how many rebuilds this URL has had since it last
    // played. All per URL: a new URL starts the count again.
    let lowLatency = false;
    let fellBack = false;
    let reloads = 0;
    // Still asking the core which route this stream is on; nothing built yet.
    let probing = true;
    // A restart or quality switch is cycling the relay (isRestartingStream).
    let frozen = false;
    const stalls = createStallReporter();

    const clearTimer = (t: number | null) => {
      if (t !== null) window.clearTimeout(t);
    };

    // Tears down the hls.js instance and everything bound to it, leaving the
    // effect alive so a rebuild can follow on the same URL.
    const teardownPlayer = () => {
      clearTimer(gateTimer);
      gateTimer = null;
      clearTimer(stopLoadTimer);
      stopLoadTimer = null;
      clearTimer(startGiveUpTimer);
      startGiveUpTimer = null;
      clearTimer(stallGiveUpTimer);
      stallGiveUpTimer = null;
      if (cushionTimer !== null) {
        window.clearInterval(cushionTimer);
        cushionTimer = null;
      }
      stalls.cancel();
      loadStopped = false;
      if (stopGovernor) {
        stopGovernor();
        stopGovernor = null;
      }
      detachVideo?.();
      detachVideo = null;
      if (hlsRef.current) {
        try {
          hlsRef.current.destroy();
        } catch {
          /* already dead */
        }
        hlsRef.current = null;
      }
    };

    const destroy = () => {
      clearTimer(reloadTimer);
      reloadTimer = null;
      teardownPlayer();
    };

    if (!video || !streamUrl || streamUrl === 'offline') {
      destroy();
      setReason(null);
      setState(streamUrl === 'offline' ? 'error' : 'idle');
      return;
    }

    if (!Hls.isSupported()) {
      // Android System WebView always has MSE; this is a diagnostics guard.
      Logger.error('[MobilePlayer] hls.js unsupported in this webview');
      setReason('unsupported');
      setState('error');
      return;
    }

    setReason(null);
    setState('loading');

    // Something the player cannot get past: fall back, rebuild, or give up
    // with the reason. Never while a restart is cycling the relay: its errors
    // are expected, and the restart brings a fresh URL anyway.
    const fail = (why: MobilePlayerFailure) => {
      if (cancelled || seq !== seqRef.current || frozen) return;
      // Same relay, whole segments: some part layouts are refused outright.
      if (lowLatency && !fellBack) {
        fellBack = true;
        Logger.warn(`[MobilePlayer] low-latency playback failed (${why}); falling back to whole segments`);
        teardownPlayer();
        setState('loading');
        build(false);
        return;
      }
      if (reloads < MAX_RELOADS) {
        reloads += 1;
        const delay = reloads * 1000;
        Logger.warn(`[MobilePlayer] playback failed (${why}); reload ${reloads} of ${MAX_RELOADS} in ${delay} ms`);
        teardownPlayer();
        setState('loading');
        reloadTimer = window.setTimeout(() => {
          reloadTimer = null;
          if (cancelled || seq !== seqRef.current || frozen) return;
          build(lowLatency);
        }, delay);
        return;
      }
      Logger.warn(`[MobilePlayer] playback failed (${why}) after ${MAX_RELOADS} reloads; showing the error`);
      teardownPlayer();
      setReason(why);
      setState('error');
    };

    const build = (ll: boolean) => {
      lowLatency = ll;
      const current = settingsRef.current;
      // The viewer's chosen distance behind live. On the parts path the edge
      // trails the broadcaster by a measured delay, so the edge target sits
      // that much lower to land on what they actually asked for.
      // The phone has no prefetch probe yet, so a whole-segment ride is
      // treated as a normal-latency broadcast (the safe side).
      const llTargetDisplayed = resolveLiveEdgeGap(current?.ll_target_latency, ll ? 'll' : 'plain');
      const llTargetRaw = ll ? edgeTargetForGap(llTargetDisplayed) : llTargetDisplayed;

      const hls = new Hls({
        enableWorker: true,
        lowLatencyMode: ll,
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
        liveSyncDuration: llTargetRaw + linkCushion(),
        liveMaxLatencyDuration: 60,
        // The core owns the stall reaction (link_adapt's cushion), so hls.js's
        // own one-second-per-stall ramp stays off, as on the desktop.
        liveSyncOnStallIncrease: 0,
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
      // Every handler below belongs to this build only; a rebuild on the same
      // URL replaces the instance, and the old one's late events must not
      // reach the new one.
      const live = () => seq === seqRef.current && hlsRef.current === hls;

      // The core judges the link by the segments this player downloads itself.
      // Parts come from the relay over loopback, so they say nothing about it.
      if (!ll) {
        hls.on(Hls.Events.FRAG_LOADED, (_event, data) => {
          if (!live() || data.part) return;
          const { loaded, loading } = data.frag.stats;
          reportSegment(loaded, loading.end - loading.start);
        });
      }
      // Where the cushion lives in hls.js (its start and resync position, and
      // the whole-segment governor's target), kept in step with the core.
      cushionTimer = window.setInterval(() => {
        if (!live()) return;
        hls.config.liveSyncDuration = llTargetRaw + linkCushion();
      }, 2000);

      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        if (!live()) return;
        // Only the low-latency path starts here. hls.js positions the playhead
        // itself on that path, so there is nothing to wait for. The ordinary
        // path starts from the buffer gate below.
        if (!ll) return;
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
      const GATE_THRESHOLD = ll ? 1 : 3;

      hls.on(Hls.Events.FRAG_BUFFERED, () => {
        if (!live()) return;
        if (playStarted || ll) return;

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
      if (!ll) {
        gateTimer = window.setTimeout(() => {
          gateTimer = null;
          if (playStarted || !live()) return;
          playStarted = true;
          Logger.warn('[MobilePlayer] buffer gate timed out, starting anyway');
          video.play().catch(() => {});
        }, 5000);
      }

      // The first frame has to come eventually; a start that never errors and
      // never plays is still a failed start.
      // A start the app was sent to the background during is judged once it
      // is back on screen, not while a hidden WebView holds playback.
      let firstFrame = false;
      const armStartGiveUp = () => {
        startGiveUpTimer = window.setTimeout(() => {
          startGiveUpTimer = null;
          if (!live() || firstFrame) return;
          if (document.visibilityState === 'hidden') {
            armStartGiveUp();
            return;
          }
          fail('timeout');
        }, START_GIVE_UP_MS);
      };
      armStartGiveUp();

      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (!live()) return;
        if (!data.fatal) return;
        if (recoverFromSequenceMismatch(hls, data)) {
          Logger.warn('[MobilePlayer] playlist refresh rejected for a sequence mismatch; accepting the next refresh');
          return;
        }
        Logger.warn('[MobilePlayer] fatal hls error:', data.type, data.details);
        fail(
          data.type === Hls.ErrorTypes.NETWORK_ERROR
            ? 'network'
            : data.type === Hls.ErrorTypes.MEDIA_ERROR
              ? 'media'
              : 'other',
        );
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
        if (!loadStopped || frozen) return;
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

      const endStallWatch = () => {
        clearTimer(stallGiveUpTimer);
        stallGiveUpTimer = null;
      };

      const onPlayIntent = () => {
        if (!live()) return;
        resumeLoad('play');
      };

      const onPlaying = () => {
        if (!live()) return;
        // Belt and braces: if anything resumed playback without a `play` event
        // reaching us, do not leave the loader stopped under a running element.
        resumeLoad('playing');
        stalls.end();
        endStallWatch();
        if (!firstFrame) {
          firstFrame = true;
          clearTimer(startGiveUpTimer);
          startGiveUpTimer = null;
        }
        // Playing again earns the next failure its full set of reloads.
        reloads = 0;
        setState('playing');
        reportState('playing');
      };
      const onWaiting = () => {
        if (!live()) return;
        // Spinner only. The earning gate is deliberately untouched here: a
        // rebuffer is not a pause.
        Logger.debug('[MobilePlayer] buffer stall (earning gate unchanged)');
        // A seek or the first load is not the link's doing.
        if (!video.paused && !video.seeking && video.currentTime > 0) stalls.begin();
        setState('stalled');
        // A rebuffer that never moves on is a dead stream. Judged by the
        // playhead, not by a timer alone: a backgrounded WebView fires timers
        // late, and a stall that recovered meanwhile must not count.
        if (stallGiveUpTimer === null && firstFrame) {
          const at = video.currentTime;
          stallGiveUpTimer = window.setTimeout(() => {
            stallGiveUpTimer = null;
            if (!live() || video.paused || video.currentTime > at + 0.5) return;
            fail('stall');
          }, STALL_GIVE_UP_MS);
        }
      };
      const onPauseOrEnd = () => {
        if (!live()) return;
        stalls.cancel();
        endStallWatch();
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
            if (!live() || !video.paused || loadStopped) return;
            loadStopped = true;
            try {
              hls.stopLoad();
              Logger.debug('[MobilePlayer] paused past the grace window, segment loading stopped');
            } catch {
              /* player already torn down */
            }
          }, STOP_LOAD_AFTER_PAUSE_MS);
        }

        // A pause that arrives while the page is already hidden is a real pause
        // (backgrounding leaves the audio alone, and in picture-in-picture the
        // native layer resumes the WebView on purpose), so it counts at once.
        reportState(document.visibilityState === 'hidden' ? 'paused_now' : 'paused');
      };
      const onVisibilityChange = () => {
        if (!live()) return;
        // The other ordering: pause landed first, then the app went to the
        // background. Rust counts a pending pause at once on hide.
        if (document.visibilityState === 'hidden') reportState('hidden');
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
      stopGovernor = ll
        ? startLatencyGovernor(hls, video, {
            label: 'mobile-ll',
            // Read live rather than captured, so moving the gap slider applies
            // without rebuilding the player.
            // Both in seconds behind the broadcaster as heard, like the desktop.
            // Plus the core's extra cushion after stalls on a poor link.
            latencyTarget: () => resolveLiveEdgeGap(settingsRef.current?.ll_target_latency, 'll') + linkCushion(),
            getLatency: () =>
              typeof hls.latency === 'number' && hls.latency > 0 ? behindLiveFromEdge(hls.latency) : null,
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
    };

    // A restart or quality switch raises isRestartingStream while the core
    // stops and restarts the relay. The player stops pulling from it and
    // pauses for that stretch, as the desktop's does, instead of polling a
    // relay that is going away. When the flag drops with a new URL this effect
    // rebuilds anyway; when it drops on the same URL (the switch failed and
    // the old relay is still what plays), loading and playback resume.
    // A zustand listener runs inside set(), before React re-renders, so a
    // write that changes the URL too is left to the rebuild.
    const unsubscribeFreeze = useAppStore.subscribe((s, prev) => {
      if (s.isRestartingStream === prev.isRestartingStream) return;
      if (cancelled || seq !== seqRef.current || s.streamUrl !== streamUrl) return;
      const hls = hlsRef.current;
      if (s.isRestartingStream) {
        frozen = true;
        clearTimer(stopLoadTimer);
        stopLoadTimer = null;
        clearTimer(stallGiveUpTimer);
        stallGiveUpTimer = null;
        if (hls) {
          loadStopped = true;
          try {
            hls.stopLoad();
          } catch {
            /* player already torn down */
          }
        }
        video.pause();
        Logger.debug('[MobilePlayer] relay restarting; loading stopped and paused');
        return;
      }
      frozen = false;
      if (hls) {
        // The play intent resumes the loader (loadStopped is set above).
        video.play().catch(() => {});
      } else if (!probing && reloadTimer === null) {
        // A failure or reload was waiting on the freeze; start over cleanly.
        setReason(null);
        setState('loading');
        build(lowLatency);
      }
    });

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
      // Extra distance from the edge the core keeps after stalls on a poor
      // link (Rust link_adapt); zero on a good one.
      await refreshLinkCushion();
      // A newer stream (or teardown) owns the element now; building here would
      // leave a zombie player attached to it.
      if (cancelled || seq !== seqRef.current) return;
      probing = false;
      build(isLowLatencyChannel);
    })();

    return () => {
      cancelled = true;
      unsubscribeFreeze();
      // Cancel a pending pause, report nothing: a paused report that lands
      // after the player is gone wrongly gates the NEXT stream's earning off,
      // and a restart, a quality switch and an ad pivot swap streamUrl while
      // deliberately leaving drops monitoring armed on the same channel. Real
      // teardown needs no help here: the stop clears the heartbeat with drops
      // monitoring.
      reportState('teardown');
      if (seq === seqRef.current) setActiveVideo(null);
      destroy();
    };
    // videoRef identity is stable (a ref); streamUrl drives the lifecycle.
    // Settings are read through a ref on purpose, so changing one does not
    // rebuild a running player.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamUrl]);

  return { state, reason };
}
