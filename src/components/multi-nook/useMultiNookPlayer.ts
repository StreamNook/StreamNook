import { useEffect, useRef, useState, useCallback } from 'react';
import Hls from 'hls.js';
import Plyr from 'plyr';
// Plyr's stylesheet ships ONCE, from globals.css, and its position there is
// load-bearing: the app's `.video-player-container` overrides beat Plyr's own
// selectors on equal specificity by SOURCE ORDER. A second copy here rode the
// lazy MultiNook chunk, which loads after globals, so vendor styling won on
// every tile (the control bar reverted to Plyr's gradient plus a 35px top pad,
// and the range tracks brightened). See the note at the top of globals.css.
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { claimPlayerFullscreen, handOffPlayerFullscreen, syncTauriWindowFullscreen } from '../../utils/windowFullscreen';
import { startLatencyGovernor } from '../../utils/liveLatencyGovernor';
import { createLiveEdgeTracker } from '../../utils/liveEdge';
import { injectPlyrControl } from '../../utils/plyrControls';
import { LIVE_BUTTON_HTML, paintLiveButton, type LiveButtonState } from '../../utils/liveButton';
import { onTileTick } from './tileTicker';
import { alignTile, isSynced, registerSyncTile, syncDelayFor } from './tileSync';

interface UseMultiNookPlayerProps {
  streamUrl?: string; // Proxy URL
  streamId: string;
  volume: number;
  muted: boolean;
  isMinimized: boolean;
}

export const useMultiNookPlayer = ({
  streamUrl,
  streamId,
  volume,
  muted,
  isMinimized,
}: UseMultiNookPlayerProps) => {
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const playerRef = useRef<Plyr | null>(null);
  const userInitiatedPauseRef = useRef<boolean>(false);
  // Stops the live-latency governor for the current tile's hls instance.
  const latencyGovernorStopRef = useRef<(() => void) | null>(null);
  const currentSettings = useAppStore(state => state.settings.video_player);
  const [isPlaying, setIsPlaying] = useState(false);
  const [isBuffering, setIsBuffering] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showControls, setShowControls] = useState(false);
  /** Per-tile smoothed distance from the live edge (see utils/liveEdge). */
  const liveEdgeRef = useRef(createLiveEdgeTracker());
  // The LIVE control in this tile's bar: red at the edge, grey behind, and a
  // click back to the edge. A tile holds only seconds of past video (tiny
  // per-tile buffers, for RAM), so it has no scrub bar; this is its timeline.
  const liveBtnRef = useRef<HTMLButtonElement | null>(null);
  const catchingUpUntilRef = useRef(0);
  const goLiveRef = useRef<() => void>(() => {});
  
  // Handlers for cleanup
  const onPlayingRef = useRef<(() => void) | null>(null);
  const onWaitingRef = useRef<(() => void) | null>(null);
  const onNativeLoadedMetadataRef = useRef<(() => void) | null>(null);

  // The painter below runs on the shared tile clock with no deps, so props it
  // needs to read have to arrive through a ref or it keeps a stale value.
  const isMinimizedRef = useRef(isMinimized);
  isMinimizedRef.current = isMinimized;

  // Paint the LIVE control at 4 Hz on the clock every tile shares (see
  // tileTicker). Docked tiles skip; a hidden window skips the tick itself.
  const paintLive = useCallback(() => {
    const video = videoRef.current;
    const btn = liveBtnRef.current;
    if (!video || !btn || !btn.isConnected || isMinimizedRef.current) return;
    // Smoothed, with hysteresis: the raw distance to the edge is a sawtooth a
    // whole segment wide. Same tracker the solo player uses (utils/liveEdge).
    const atEdge = !liveEdgeRef.current.isBehind(video) && !video.paused && video.readyState >= 3;
    let state: LiveButtonState = atEdge ? 'live' : 'behind';
    if (atEdge) catchingUpUntilRef.current = 0;
    else if (catchingUpUntilRef.current > performance.now()) state = 'catching-up';
    paintLiveButton(btn, state);
  }, []);
  useEffect(() => onTileTick(paintLive), [paintLive]);

  // Back to the edge: as far forward as hls.js says is safe, within what is
  // buffered, and playing. While a Resync holds the tiles together, "live" is
  // the shared delay instead, so the click rejoins the others.
  useEffect(() => {
    goLiveRef.current = () => {
      const video = videoRef.current;
      if (!video) return;
      if (isSynced()) {
        const align = () => alignTile(streamId, true);
        if (video.paused) video.play().then(align).catch(() => { /* autoplay policy / teardown */ });
        else align();
        catchingUpUntilRef.current = performance.now() + 6000;
        const btn = liveBtnRef.current;
        if (btn && btn.isConnected) paintLiveButton(btn, 'catching-up');
        return;
      }
      const b = video.buffered;
      const bufferedEnd = b.length > 0 ? b.end(b.length - 1) : 0;
      const pos = hlsRef.current?.liveSyncPosition;
      const syncPos = pos != null && Number.isFinite(pos) ? pos : Infinity;
      const target = Math.min(syncPos, bufferedEnd - 1);
      if (Number.isFinite(target) && target > video.currentTime + 0.25) video.currentTime = target;
      if (video.paused) video.play().catch(() => { /* autoplay policy / teardown */ });
      catchingUpUntilRef.current = performance.now() + 6000;
      const btn = liveBtnRef.current;
      if (btn && btn.isConnected) paintLiveButton(btn, 'catching-up');
    };
  }, [streamId]);

  // Put the LIVE control in the bar once Plyr has built it, right after play.
  const attachLiveButton = useCallback(() => {
    const container = (playerRef.current as unknown as { elements?: { container?: HTMLElement } } | null)?.elements?.container;
    if (!container) return;
    injectPlyrControl(container, {
      attr: 'data-streamnook-live',
      className: 'sn-live-btn',
      html: LIVE_BUTTON_HTML,
      onClick: () => goLiveRef.current(),
      place: (controls) => {
        const play = controls.querySelector(':scope > [data-plyr="play"]');
        return play ? { after: play } : null;
      },
      onInserted: (btn) => {
        liveBtnRef.current = btn;
        paintLiveButton(btn, 'behind');
      },
    });
  }, []);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    // Apply volume/mute to native video if plyr isn't ready
    if (playerRef.current) {
      // Only apply if changed to prevent Plyr from implicitly unmuting on volume assignments
      if (typeof volume === 'number' && playerRef.current.volume !== volume) {
        playerRef.current.volume = volume;
      }
      if (typeof muted === 'boolean' && playerRef.current.muted !== muted) {
        playerRef.current.muted = muted;
      }
    } else {
      if (video.volume !== volume) video.volume = volume;
      if (video.muted !== muted) video.muted = muted;
    }
  }, [volume, muted]);

  useEffect(() => {
    if (!playerRef.current) return;
    
    // Explicitly mute/restore volume when toggling dock state
    if (isMinimized) {
      if (!playerRef.current.muted) playerRef.current.muted = true;
    } else {
      if (playerRef.current.volume !== volume) playerRef.current.volume = volume;
      if (playerRef.current.muted !== muted) playerRef.current.muted = muted;
    }
  }, [isMinimized, muted, volume]); // Added muted, volume to deps for correct restoration

  // Clean up Plyr on unmount
  useEffect(() => {
    return () => {
      if (playerRef.current) {
        handOffPlayerFullscreen(playerRef.current, { nextPlayerComing: true });
        playerRef.current.destroy();
        playerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !streamUrl) return;

    Logger.debug(`[MultiNook-${streamId}] Initializing player with URL: ${streamUrl}`);
    
    // Avoid synchronous setState in effect
    queueMicrotask(() => {
      setIsBuffering(true);
      setError(null);
    });

    // Refs to store actual listener handlers for cleanup
    onPlayingRef.current = null;
    onWaitingRef.current = null;
    onNativeLoadedMetadataRef.current = null;

    // Destroy existing HLS instance
    if (hlsRef.current) {
      hlsRef.current.destroy();
      hlsRef.current = null;
    }

    const onNativeError = () => {
      if (!Hls.isSupported() && video?.error) {
          setError('Failed to load video (Native)');
      }
    };

    if (video) {
        // Fallback for native Safari playback
        video.addEventListener('error', onNativeError);
    }

    // The tile's relay tags its proxy URL with `ll=1` when the per-tile LL-HLS
    // origin activated (low-latency broadcast). The mode must be chosen at hls.js
    // construction, and the flag rides the URL this effect already keys on, so a
    // refreshed URL always carries the matching mode with no extra round trip.
    const isLowLatencyChannel = streamUrl.includes('&ll=1');
    // The tile's own distance from live (see liveSyncDuration below). A tile
    // that starts while a Resync holds the others starts at their delay
    // instead, so it joins them without a jump.
    const cushion = isLowLatencyChannel ? 3 : 8;
    const startDelay = Math.max(cushion, syncDelayFor(streamId) ?? 0);
    let unregisterSync: (() => void) | null = null;

    if (Hls.isSupported()) {
      const hls = new Hls({
        debug: false,
        enableWorker: true,
        lowLatencyMode: isLowLatencyChannel, // True only when the tile's relay serves the LL-HLS origin (blocking reload + parts); hls.js's native LL engine owns pacing there. Otherwise false: native LL parsing against a plain proxied playlist causes cyclic starvation.
        startFragPrefetch: false, // Off for tiles: prefetch double-buffers TS chunks in the V8 heap, multiplied across every tile.
        // Per-tile buffers are bounded well below the solo player's. A MultiNook
        // grid runs many hls.js instances at once, and each full 60 MB / 120s
        // buffer multiplies across every tile (9 tiles at 60 MB is ~540 MB of
        // video buffer alone). Grid tiles don't need a DVR scrub window (the solo
        // player keeps the generous buffer for that), so a small forward/back
        // buffer is plenty here. A uniform low cap also beats tiering one tile
        // high: 9 small tiles use less total RAM than 1 large + 8 small.
        backBufferLength: 10,
        maxBufferLength: 15,
        maxMaxBufferLength: 30,
        maxBufferSize: 16 * 1000 * 1000,
        maxBufferHole: 0.5, 
        highBufferWatchdogPeriod: 2, 
        nudgeOffset: 0.2, 
        nudgeMaxRetry: 3, 
        maxFragLookUpTolerance: 0.5, 
        liveSyncDuration: startDelay, // `cushion`, or a Resync's shared delay when larger. LL origin: parts are consumed progressively, so tiles can ride near the edge; 3 keeps one segment of headroom over the solo player's 2 because per-tile buffers are tiny. Non-LL: conservative 8s BY POLICY. Grid tiles run deliberately tiny per-tile buffers (maxBufferLength 15) for RAM, so they can't absorb a normal ~3s Twitch segment-delivery gap at a tight cushion on the whole-segment path — 6 stalled in the wild.
        liveMaxLatencyDuration: 600, // Massive drift ceiling so manual scrobbling backwards into the DVR buffer isn't violently snapped to live edge.
        maxLiveSyncPlaybackRate: 1, // hls.js's latency controller is fully inert on every path (its 0.05-quantized rate steps are audible pops on music); the latency governor below owns catch-up instead.
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
        startLevel: currentSettings.start_quality || -1, 
        abrEwmaDefaultEstimate: 3_000_000, 
        abrEwmaFastLive: 3.0, 
        abrEwmaSlowLive: 9.0, 
        abrBandWidthFactor: 0.95, 
        abrBandWidthUpFactor: 0.7, 
      });

      hlsRef.current = hls;

      // Continuous live-latency maintenance, same forward-buffer governor as the solo
      // player with a slightly wider band for tiles (tiny per-tile buffers, and grid
      // latency isn't perceptually important). It only consumes excess buffer, so it
      // can't starve a tile. Target is read from hls.config.liveSyncDuration. Owns
      // catch-up on BOTH paths (hls.js's controller is disabled above); LL tiles get
      // the gentle ramped profile so rate changes never pop tile audio.
      if (latencyGovernorStopRef.current) {
        latencyGovernorStopRef.current();
        latencyGovernorStopRef.current = null;
      }
      // While a Resync holds the tiles together (see tileSync), the governor
      // holds the shared delay behind live instead of its own cushion; the
      // getter returns null outside a sync, which is the cushion mode above.
      const syncHold = {
        getTarget: () => cushion,
        latencyTarget: () => syncDelayFor(streamId),
        getLatency: () => (Number.isFinite(hls.latency) && hls.latency > 0 ? hls.latency : null),
      };
      latencyGovernorStopRef.current = isLowLatencyChannel
        ? startLatencyGovernor(hls, video, {
            label: `tile-ll ${streamId}`,
            ceiling: 1.03,
            band: 1.0,
            // Tiles run tiny buffers; the slow side rides out delivery wobbles
            // that would otherwise stall the tile.
            floor: 1.0,
            slowRate: 0.97,
            tickMs: 500,
            rampStep: 0.01,
            ...syncHold,
            log: Logger.debug,
          })
        : startLatencyGovernor(hls, video, {
            label: `tile ${streamId}`,
            // A whole segment of slack holding the cushion; a tighter second
            // holding a shared delay, where latency moves smoothly.
            band: () => (isSynced() ? 1.0 : 2.0),
            // Never overspeed a thin buffer: a playlist that stops refreshing
            // inflates the measured delay while playback is fine.
            floor: 1.5,
            ...syncHold,
            log: Logger.debug,
          });
      unregisterSync = registerSyncTile(streamId, {
        hls,
        video,
        lowLatency: isLowLatencyChannel,
        cushion,
        onSeek: (delta) => liveEdgeRef.current.shift(delta),
      });

      let playStarted = false;
      let fragsBuffered = 0;

      const startPlayback = () => {
        if (playStarted) return;
        playStarted = true;
        video.play().catch((e) => {
          Logger.debug(`[MultiNook-${streamId}] Autoplay failed:`, e);
          video.muted = true;
          video.play().catch(() => {});
        });
        setIsBuffering(false);
      };

      hls.loadSource(streamUrl);
      hls.attachMedia(video);

      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        Logger.debug(`[MultiNook-${streamId}] Manifest parsed, starting playback`);
        
        // Initialize Plyr once Media is attached
        if (!playerRef.current) {
          playerRef.current = new Plyr(video, {
            controls: ['play', 'volume', 'settings', 'fullscreen'],
            settings: ['speed'], // Quality submenu is injected manually by MultiNookCell (every tile)
            autoplay: false, // Wait for buffer gate
            muted: muted,
            clickToPlay: false, // Disabled so we can capture clicks for focus
            // Force Plyr's CSS-only fullscreen and bridge it to the Tauri window's
            // true OS fullscreen (see syncTauriWindowFullscreen). Without this a
            // tile's fullscreen only fills the borderless window up to the taskbar.
            fullscreen: { enabled: true, fallback: 'force', iosNative: false },
            storage: { enabled: false }
          });

          playerRef.current.on('enterfullscreen', () => syncTauriWindowFullscreen(true));
          playerRef.current.on('exitfullscreen', () => syncTauriWindowFullscreen(false));
          claimPlayerFullscreen(playerRef.current);
          attachLiveButton();

          // Override duration for live stream progress bar
          Object.defineProperty(video, 'duration', {
            get: function () {
              const buffered = this.buffered;
              if (buffered.length > 0) {
                return buffered.end(buffered.length - 1);
              }
              return Infinity;
            },
            configurable: true,
          });
          
          if (typeof volume === 'number') playerRef.current.volume = volume;
          if (typeof muted === 'boolean') playerRef.current.muted = muted;

          // Listen for pause to know if it was user initiated
          playerRef.current.on('play', () => {
             userInitiatedPauseRef.current = false;
          });
          playerRef.current.on('pause', () => {
             setTimeout(() => {
                if (video.paused) {
                   userInitiatedPauseRef.current = true;
                }
             }, 50);
          });

          // Sync backwards to store
          playerRef.current.on('volumechange', () => {
            if (!playerRef.current) return;
            // Prevent syncing changes if we are minimized (docked streams are forced mute)
            const store = usemultiNookStore.getState();
            const currentState = store.slots.find(s => s.id === streamId);
            if (currentState?.isMinimized) return;

            // Toolbar mute-all forces the player muted without touching slot.muted
            // (that state is what unmute-all restores), so never sync the forced
            // mute back. A hands-on unmute of this tile while mute-all is engaged
            // is an explicit break-out: lift the global mute and sync normally.
            if (store.isAllMuted) {
              if (playerRef.current.muted) return;
              store.setAllMuted(false);
            }

            const newVol = playerRef.current.volume;
            const newMuted = playerRef.current.muted;
            usemultiNookStore.getState().updateSlot(streamId, { volume: newVol, muted: newMuted });
          });

          playerRef.current.on('controlsshown', () => setShowControls(true));
          playerRef.current.on('controlshidden', () => setShowControls(false));

          // Initial state
          setShowControls(true);
        }

        if (isLowLatencyChannel) {
          // LL path: hls.js owns the start position (liveSyncDuration back from the
          // part edge) and FRAG_BUFFERED doesn't fire per part, so the cushion gate
          // below would only time out and start late. Same rule as the solo player.
          startPlayback();
        }
        // Non-LL: do not force play() here to prevent cold-start stall.
        // Wait for FRAG_BUFFERED gate.
      });

      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (!data.fatal) {
           if (data.details === 'bufferStalledError') {
             Logger.debug(`[MultiNook-${streamId}] Buffer stalled, attempting recovery...`);
             if (video.paused && !userInitiatedPauseRef.current) {
               video.play().catch(() => {});
             }
             // NON-LL tiles only: a forward seek toward the edge on the LL
             // path is the documented mid-playback freeze. LL tiles ride the
             // governor + cushion instead (no in-buffer snap on tiles).
             if (!isLowLatencyChannel) {
               const buffered = video.buffered;
               if (buffered.length > 0) {
                 const currentTime = video.currentTime;
                 const bufferedEnd = buffered.end(buffered.length - 1);
                 if (bufferedEnd - currentTime > 2.0) {
                   video.currentTime = currentTime + 0.5;
                 }
               }
             }
           }
           return;
        }

        if (data.fatal) {
          Logger.error(`[MultiNook-${streamId}] Fatal error:`, data);
          switch (data.type) {
            case Hls.ErrorTypes.NETWORK_ERROR:
              setError('Network error');
              hls.startLoad();
              break;
            case Hls.ErrorTypes.MEDIA_ERROR:
              setError('Media error');
              hls.recoverMediaError();
              break;
            default:
              setError('Playback error');
              hls.destroy();
              break;
          }
        }
      });
      
      // Build a small startup cushion before playing instead of starting on the
      // very first fragment. Playing on one ~2s fragment means the buffer drains
      // ~2s later if the next segment isn't ready yet, which is exactly what
      // happens when a preset cold-starts many proxies at once (they compete for
      // bandwidth and the relay is cold), producing a buffer stall right after the
      // stream "loads". Waiting for ~a couple seconds of buffer rides over that
      // cold-start gap. The cushion is measured in seconds so it adapts to the
      // stream's segment length, and the frag-count cap keeps the wait bounded so
      // it never hangs on the loading spinner. Non-LL only: the LL path starts in
      // MANIFEST_PARSED above.
      //
      // Three frags, not four: only a source with segments shorter than about
      // 1.2 s reaches the cap before the seconds do, and that is TikTok's relay,
      // whose one second segments already come with three seconds held back
      // before the first playlist answers. A fourth frag only cost it a second.
      const START_CUSHION_SECONDS = 3.5;
      const MAX_STARTUP_FRAGS = 3;

      if (!isLowLatencyChannel) {
        hls.on(Hls.Events.FRAG_BUFFERED, () => {
          if (playStarted) return;
          fragsBuffered += 1;
          const b = video.buffered;
          const bufferedDur = b.length > 0 ? b.end(b.length - 1) - b.start(0) : 0;
          if (bufferedDur >= START_CUSHION_SECONDS || fragsBuffered >= MAX_STARTUP_FRAGS) {
            Logger.debug(
              `[MultiNook-${streamId}] Startup cushion ready (${bufferedDur.toFixed(1)}s over ${fragsBuffered} frags), starting playback`,
            );
            startPlayback();
          }
        });
      }

      let joinedSync = false;
      const onPlaying = () => {
        setIsPlaying(true);
        setIsBuffering(false);
        setError(null);
        // A tile that starts during a Resync settles onto the shared delay
        // once, as it begins.
        if (!joinedSync) {
          joinedSync = true;
          alignTile(streamId);
        }
      };

      const onWaiting = () => setIsBuffering(true);
      
      onPlayingRef.current = onPlaying;
      onWaitingRef.current = onWaiting;

      video.addEventListener('playing', onPlaying);
      video.addEventListener('waiting', onWaiting);

    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
      // Safari fallback
      video.src = streamUrl;
      const onNativeLoadedMetadata = () => {
        if (!playerRef.current) {
          playerRef.current = new Plyr(video, {
            controls: ['play', 'volume', 'settings', 'fullscreen'],
            settings: ['speed'], // Quality submenu is injected manually by MultiNookCell (every tile)
            autoplay: false,
            muted: muted,
            clickToPlay: false, // Disabled so we can capture clicks for focus
            // Force Plyr's CSS-only fullscreen and bridge it to the Tauri window's
            // true OS fullscreen (see syncTauriWindowFullscreen). Without this a
            // tile's fullscreen only fills the borderless window up to the taskbar.
            fullscreen: { enabled: true, fallback: 'force', iosNative: false },
            storage: { enabled: false }
          });

          playerRef.current.on('enterfullscreen', () => syncTauriWindowFullscreen(true));
          playerRef.current.on('exitfullscreen', () => syncTauriWindowFullscreen(false));
          claimPlayerFullscreen(playerRef.current);
          attachLiveButton();

          Object.defineProperty(video, 'duration', {
            get: function () {
              const buffered = this.buffered;
              if (buffered.length > 0) {
                return buffered.end(buffered.length - 1);
              }
              return Infinity;
            },
            configurable: true,
          });

          if (typeof volume === 'number') playerRef.current.volume = volume;
          if (typeof muted === 'boolean') playerRef.current.muted = muted;

          // Sync backwards to store
          playerRef.current.on('volumechange', () => {
            if (!playerRef.current) return;
            const store = usemultiNookStore.getState();
            const currentState = store.slots.find(s => s.id === streamId);
            if (currentState?.isMinimized) return;

            // Same mute-all guard as the hls path: never sync the forced mute
            // back; a hands-on unmute of this tile breaks out of mute-all.
            if (store.isAllMuted) {
              if (playerRef.current.muted) return;
              store.setAllMuted(false);
            }

            const newVol = playerRef.current.volume;
            const newMuted = playerRef.current.muted;
            usemultiNookStore.getState().updateSlot(streamId, { volume: newVol, muted: newMuted });
          });

          playerRef.current.on('controlsshown', () => setShowControls(true));
          playerRef.current.on('controlshidden', () => setShowControls(false));
          
          setShowControls(true);
        }
        video.play().catch(e => Logger.error(`[MultiNook-${streamId}] Fallback auto-play failed:`, e));
      };

      onNativeLoadedMetadataRef.current = onNativeLoadedMetadata;
      video.addEventListener('loadedmetadata', onNativeLoadedMetadata);

    }

    return () => {
      if (video) {
        video.removeEventListener('error', onNativeError);
        if (onPlayingRef.current) video.removeEventListener('playing', onPlayingRef.current);
        if (onWaitingRef.current) video.removeEventListener('waiting', onWaitingRef.current);
        if (onNativeLoadedMetadataRef.current) video.removeEventListener('loadedmetadata', onNativeLoadedMetadataRef.current);
      }
      unregisterSync?.();
      if (latencyGovernorStopRef.current) {
        latencyGovernorStopRef.current();
        latencyGovernorStopRef.current = null;
      }
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
    };
  }, [streamUrl, streamId, attachLiveButton]); // intentionally omitting volume/muted from deps

  return {
    videoRef,
    playerRef,
    isPlaying,
    isBuffering,
    error,
    showControls,
  };
};

