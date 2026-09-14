// Touch-first player surface: the video element plus a tap-driven glass control
// overlay. No Plyr; hls.js runs via useMobileHlsEngine.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowClockwise, ArrowsOut, Bell, BellSlash, Columns, Eye, Gear, Pause, PictureInPicture, Play, Rows, ShareNetwork, SpeakerHigh, SpeakerSlash } from 'phosphor-react';
import { setPipMuted, shareText } from '../nativeBridge';
import { toggleChannelMuted } from '../notifyChannels';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { buildShareUrl } from '../../utils/shareLink';
import { useAppStore } from '../../stores/AppStore';
import { useMobileHlsEngine } from './useMobileHlsEngine';
import { setLiveVideo } from './liveVideo';
import { attachLockScreenAudio, releaseLockScreenAudio } from './lockScreenAudio';
import { QualitySheet } from './QualitySheet';
import PenroseMarch from '../../components/PenroseMarch';

const CONTROLS_HIDE_MS = 3000;

// "2h 14m" since the broadcast started, refreshed while the overlay is up.
function formatUptime(startedAt: string | undefined, nowMs: number): string {
  if (!startedAt) return '';
  const start = Date.parse(startedAt);
  if (!Number.isFinite(start)) return '';
  const totalSec = Math.max(0, Math.floor((nowMs - start) / 1000));
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

export const MobilePlayer: React.FC<{
  /** Landscape immersive mode: overlay carries no bottom rounding, adds insets. */
  immersive?: boolean;
  onToggleFullscreen?: () => void;
  /** Shrink into the floating player. In-app that is the mini box; leaving the
   *  app hands the same thing to the OS as system picture-in-picture. */
  onMinimize?: () => void;
  /** Mini/PiP presentation: video only, no overlay chrome. */
  compact?: boolean;
  /** Current big-screen arrangement, for the toggle's icon and label. */
  layoutMode?: 'columns' | 'stacked';
  /** Set only where both arrangements fit, which is what gates the control. */
  onToggleLayout?: () => void;
}> = ({
  immersive = false,
  onToggleFullscreen,
  onMinimize,
  compact = false,
  layoutMode,
  onToggleLayout,
}) => {
  const videoRef = useRef<HTMLVideoElement>(null);
  const { state } = useMobileHlsEngine(videoRef);
  const restartStream = useAppStore((s) => s.restartStream);
  const currentStream = useAppStore((s) => s.currentStream);
  // Subscribed rather than read through the helper, so toggling the bell
  // re-renders this overlay instead of leaving a stale icon behind.
  const mutedChannels = useAppStore((s) => s.settings.live_notifications?.muted_live_channels);
  const alertsOff = !!currentStream && (mutedChannels ?? []).includes(currentStream.user_login.toLowerCase());

  const [controlsVisible, setControlsVisible] = useState(true);
  const [paused, setPaused] = useState(false);
  const [muted, setMuted] = useState(false);
  const [qualityOpen, setQualityOpen] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // The loading logo fades out over the first playing frames instead of
  // vanishing on the exact tick the state flips — that hard swap was half of
  // the "logo snaps into the video" jank. `idle` counts as loading because the
  // engine sits idle until the stream URL resolves, which is precisely the
  // stretch the logo exists to cover. Unmounted after the fade so the logo's
  // animation loop is not left running invisibly under playback.
  // Low Latency takes effect at the origin probe that runs when a stream
  // STARTS, so flipping it mid-stream changed nothing until the next open.
  // Desktop restarts the stream on the flip (VideoPlayer.tsx); mobile shares
  // that settings panel but had no equivalent, so the toggle looked inert.
  // Ref-compared rather than first-run-flagged for the same reason desktop
  // does it: the restart remounts this component, and a flag would let the
  // remount read as another toggle and loop.
  const lowLatency = useAppStore((s) => s.settings.video_player?.experimental_low_latency);
  const prevLowLatency = useRef(lowLatency);
  useEffect(() => {
    if (prevLowLatency.current === lowLatency) return;
    prevLowLatency.current = lowLatency;
    // The settings panel already pushed the flag to the backend; this only has
    // to re-run the origin probe that reads it.
    if (currentStream) void restartStream();
  }, [lowLatency, currentStream, restartStream]);

  // Lock-screen audio + media controls.
  //
  // Bound here rather than in WatchScreen because this is where the <video>
  // lives, and the binding has to follow the element. Keyed on the channel so a
  // channel switch re-publishes the metadata; the helper is idempotent for the
  // re-renders in between.
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !currentStream) return;
    return attachLockScreenAudio(video, {
      title: currentStream.title || currentStream.user_name,
      artist: currentStream.user_name,
      artUrl: currentStream.profile_image_url ?? '',
    });
  }, [currentStream?.user_login, currentStream?.title, currentStream?.user_name, currentStream?.profile_image_url, currentStream]);

  // Leaving the stream entirely must drop the session. Otherwise the foreground
  // service keeps the WebView awake in the background for audio that has ended,
  // which is the exact battery cost this feature is meant to be worth paying.
  useEffect(() => () => releaseLockScreenAudio(), []);

  // Manual restart feedback: the icon spins from the tap until the reload has
  // been THROUGH loading and out the other side. A simple "clear when not
  // loading" wiped the flag on the very render the tap caused — the engine
  // still reported playing for that instant, so the spin never drew and only
  // a second tap (landing after the state flipped) ever showed it. The phase
  // must observe loading before the exit condition may clear it.
  const [restartPhase, setRestartPhase] = useState<'none' | 'kicked' | 'loading'>('none');
  if (restartPhase === 'kicked' && (state === 'loading' || state === 'idle')) {
    setRestartPhase('loading');
  } else if (restartPhase === 'loading' && state !== 'loading' && state !== 'idle') {
    setRestartPhase('none');
  }
  const restarting = restartPhase !== 'none';
  // If the restart dies before the engine ever reports loading (a failed
  // invoke), don't spin forever.
  useEffect(() => {
    if (restartPhase !== 'kicked') return;
    const t = setTimeout(() => setRestartPhase((p) => (p === 'kicked' ? 'none' : p)), 5000);
    return () => clearTimeout(t);
  }, [restartPhase]);

  const logoWanted = state === 'loading' || state === 'idle';
  const [logoMounted, setLogoMounted] = useState(true);
  // Render-time adjustment (same pattern as the drill-in screens' `last`
  // retention): re-mount instantly when loading returns, no effect round trip.
  if (logoWanted && !logoMounted) setLogoMounted(true);
  useEffect(() => {
    if (logoWanted) return;
    const t = setTimeout(() => setLogoMounted(false), 340);
    return () => clearTimeout(t);
  }, [logoWanted]);

  // Uptime ticks while the overlay chrome exists at all.
  //
  // useVisibleInterval rather than a bare setInterval, because a WebView timer
  // is throttled or frozen outright while the app is backgrounded: the clock
  // silently falls behind and then reads stale for up to a full period after
  // you come back, which is exactly when someone looks at it. This hook skips
  // ticks while hidden and fires immediately on becoming visible, so resuming
  // shows the real uptime rather than the one from whenever the app was last
  // awake.
  useVisibleInterval(() => {
    if (compact) return;
    setNowMs(Date.now());
  }, 30_000);

  const scheduleHide = useCallback(() => {
    if (hideTimer.current) clearTimeout(hideTimer.current);
    hideTimer.current = setTimeout(() => setControlsVisible(false), CONTROLS_HIDE_MS);
  }, []);

  useEffect(() => {
    scheduleHide();
    return () => {
      if (hideTimer.current) clearTimeout(hideTimer.current);
    };
  }, [scheduleHide]);

  // Mute from the system PiP window. Web content cannot draw usable controls
  // there, so the toggle is a native RemoteAction; MainActivity broadcasts back
  // into the WebView, and we push the resulting state out again so the OS can
  // flip its icon between mute and unmute.
  useEffect(() => {
    const onPipMute = () => {
      const video = videoRef.current;
      if (!video) return;
      video.muted = !video.muted;
      setMuted(video.muted);
      setPipMuted(video.muted);
    };
    window.addEventListener('sn:pip-mute', onPipMute);
    return () => window.removeEventListener('sn:pip-mute', onPipMute);
  }, []);

  const onSurfaceTap = () => {
    setControlsVisible((v) => {
      const next = !v;
      if (next) scheduleHide();
      return next;
    });
  };

  const togglePlay = (e: React.MouseEvent) => {
    e.stopPropagation();
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) {
      void video.play();
      setPaused(false);
    } else {
      video.pause();
      setPaused(true);
    }
    scheduleHide();
  };

  const toggleMute = (e: React.MouseEvent) => {
    e.stopPropagation();
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
    setMuted(video.muted);
    // Keep the PiP action's icon in step with the in-app control.
    setPipMuted(video.muted);
    scheduleHide();
  };

  return (
    <div
      className="relative w-full h-full bg-black overflow-hidden"
      onClick={onSurfaceTap}
    >
      <video
        // Callback ref so the clip sheet can duck this element's audio while a
        // clip plays. Assigns the same ref object the engine reads, then
        // publishes it; both are cleared on unmount when React passes null.
        ref={(el) => {
          (videoRef as React.MutableRefObject<HTMLVideoElement | null>).current = el;
          setLiveVideo(el);
        }}
        className="w-full h-full object-contain"
        playsInline
        autoPlay
        // A <video> with no poster gets Android WebView's own grey play-triangle
        // placeholder until the first frame decodes. A transparent poster
        // suppresses it, so what shows before playback is our logo below.
        // 16x9, not 1x1: pre-metadata the element takes its intrinsic ratio
        // from the poster, and a square one made the loading band render
        // square before lurching to 16:9 at first frame (see playerBandClass).
        poster="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 9' width='16' height='9'/%3E"
      />

      {logoMounted && (
        <div
          className={`absolute inset-0 flex items-center justify-center pointer-events-none transition-opacity duration-300 ${
            logoWanted ? 'opacity-100' : 'opacity-0'
          }`}
        >
          <PenroseMarch size={compact ? 44 : 84} />
        </div>
      )}

      {state === 'error' && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/70">
          <div className="text-sm text-white/80">Stream unavailable</div>
          <button
            onClick={(e) => {
              e.stopPropagation();
              void restartStream();
            }}
            className="glass-button px-5 py-2.5 text-sm font-semibold text-white"
          >
            Retry
          </button>
        </div>
      )}

      {/* Control overlay. Carries the persistent stream info (channel, title,
          category, viewers, uptime) so nothing has to sit between the player
          and chat, matching how the Twitch app does it. */}
      <div
        className={`absolute inset-0 transition-opacity duration-200 ${
          controlsVisible && !compact ? 'opacity-100' : 'opacity-0 pointer-events-none'
        }`}
        style={{
          background:
            'linear-gradient(to bottom, rgba(0,0,0,0.72) 0%, rgba(0,0,0,0.28) 26%, transparent 46%, transparent 62%, rgba(0,0,0,0.6) 100%)',
        }}
      >
        {/* Stream info, top-left */}
        {currentStream && (
          <div
            className="absolute inset-x-0 top-0 px-3 pt-2"
            style={immersive ? { paddingTop: 'calc(var(--sn-safe-t, 0px) + 8px)' } : undefined}
          >
            {/* Identity row: avatar + name, with the live metrics opposite. */}
            <div className="flex items-center gap-2 min-w-0">
              {currentStream.profile_image_url ? (
                <img
                  src={currentStream.profile_image_url}
                  alt=""
                  draggable={false}
                  className="w-7 h-7 rounded-full object-cover shrink-0 ring-2 ring-live/80"
                />
              ) : (
                <div className="w-7 h-7 rounded-full bg-white/15 shrink-0 flex items-center justify-center text-[12px] font-bold text-white ring-2 ring-live/80">
                  {currentStream.user_name?.charAt(0).toUpperCase()}
                </div>
              )}
              <span className="text-[13.5px] font-semibold text-white truncate">
                {currentStream.user_name}
              </span>
              {currentStream.broadcaster_type === 'partner' && (
                <svg className="w-3 h-3 shrink-0 -ml-0.5" viewBox="0 0 16 16" fill="#9146FF">
                  <path
                    fillRule="evenodd"
                    d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z"
                    clipRule="evenodd"
                  ></path>
                </svg>
              )}
              {/* Live-alert opt-out for this channel, next to who it is rather
                  than down in the playback controls: it is a fact about the
                  channel, not about this session's playback. */}
              <button
                onClick={() => void toggleChannelMuted(currentStream.user_login)}
                aria-label={
                  alertsOff
                    ? `Turn on live alerts for ${currentStream.user_name}`
                    : `Turn off live alerts for ${currentStream.user_name}`
                }
                aria-pressed={!alertsOff}
                className={`shrink-0 flex items-center justify-center p-1 transition-colors ${
                  alertsOff ? 'text-white/45' : 'text-white'
                }`}
              >
                {alertsOff ? <BellSlash size={15} /> : <Bell size={15} weight="fill" />}
              </button>
              <span className="ml-auto flex items-center gap-2 shrink-0 pl-2">
                <span className="flex items-center gap-1 text-[12px] font-medium text-live">
                  <Eye size={12} weight="fill" />
                  {currentStream.viewer_count.toLocaleString()}
                </span>
                <span className="text-[12px] text-white/70">
                  {formatUptime(currentStream.started_at, nowMs)}
                </span>
              </span>
            </div>
            {/* Title + category run the full width beneath the identity row. */}
            <div className="text-[12.5px] text-white/85 truncate leading-snug mt-1">
              {currentStream.title}
            </div>
            {currentStream.game_name && (
              <div className="text-[11.5px] text-white/60 truncate leading-snug">
                {currentStream.game_name}
              </div>
            )}
          </div>
        )}
        {/* Center play/pause */}
        <button
          onClick={togglePlay}
          className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-16 h-16 rounded-full bg-black/45 flex items-center justify-center text-white active:scale-95 transition-transform"
          aria-label={paused ? 'Play' : 'Pause'}
        >
          {paused ? <Play size={30} weight="fill" /> : <Pause size={30} weight="fill" />}
        </button>

        {/* Bottom bar */}
        <div
          className="absolute inset-x-0 bottom-0 flex items-center justify-between px-3 pb-2"
          style={immersive ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 8px)' } : undefined}
        >
          <div className="flex items-center gap-1 min-w-0">
            <span className="px-1.5 py-0.5 rounded text-[11px] font-semibold bg-live/90 text-white leading-none">
              LIVE
            </span>
          </div>
          <div className="flex items-center">
            <button
              onClick={toggleMute}
              className="sn-touch flex items-center justify-center text-white"
              aria-label={muted ? 'Unmute' : 'Mute'}
            >
              {muted ? <SpeakerSlash size={21} /> : <SpeakerHigh size={21} />}
            </button>
            {currentStream && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  // Our own share link, not a raw twitch.tv URL: the landing
                  // page hands off to StreamNook when it is installed and
                  // falls back to Twitch when it is not. buildShareUrl also
                  // carries the day tag that keeps Discord from serving a
                  // stale preview card. Same helper the desktop share uses.
                  shareText(
                    `${currentStream.user_name} is live: ${buildShareUrl(currentStream.user_login)}`,
                    currentStream.title || currentStream.user_name,
                  );
                }}
                className="sn-touch flex items-center justify-center text-white"
                aria-label="Share stream"
              >
                <ShareNetwork size={21} />
              </button>
            )}
            {onMinimize && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onMinimize();
                }}
                className="sn-touch flex items-center justify-center text-white"
                aria-label="Minimize player"
              >
                <PictureInPicture size={21} />
              </button>
            )}
            {/* The unstick button. A frozen player looks exactly like a paused
                one from the outside, so this reloads the whole pipeline via the
                same restart the error screen's Retry uses; today's loading
                crossfade makes the reload read as one smooth dip through the
                logo instead of a teardown. */}
            <button
              onClick={(e) => {
                e.stopPropagation();
                setRestartPhase('kicked');
                void restartStream();
                scheduleHide();
              }}
              className="sn-touch flex items-center justify-center text-white"
              aria-label="Restart stream"
            >
              <ArrowClockwise size={21} className={restarting ? 'animate-spin' : ''} />
            </button>
            <button
              onClick={(e) => {
                e.stopPropagation();
                setQualityOpen(true);
              }}
              className="sn-touch flex items-center justify-center text-white"
              aria-label="Quality"
            >
              <Gear size={21} />
            </button>
            {/* Only on a screen where both arrangements genuinely work, so this
                never appears as a control with one sensible setting. */}
            {onToggleLayout && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleLayout();
                }}
                className="sn-touch flex items-center justify-center text-white"
                aria-label={
                  layoutMode === 'columns' ? 'Stack player above chat' : 'Put chat beside player'
                }
              >
                {layoutMode === 'columns' ? <Rows size={21} /> : <Columns size={21} />}
              </button>
            )}
            {onToggleFullscreen && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleFullscreen();
                }}
                className="sn-touch flex items-center justify-center text-white"
                aria-label="Fullscreen"
              >
                <ArrowsOut size={21} />
              </button>
            )}
          </div>
        </div>
      </div>

      <QualitySheet open={qualityOpen} onClose={() => setQualityOpen(false)} />
    </div>
  );
};
