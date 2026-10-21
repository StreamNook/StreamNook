// Touch-first player surface: the video element plus a tap-driven glass control
// overlay. No Plyr; hls.js runs via useMobileHlsEngine.
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowClockwise, Bell, BellSlash, CaretLeft, ChatTeardropText, Columns, Gear, MoonStars, Pause, Play, Rows, Screencast, ShareNetwork, SpeakerHigh, SpeakerSlash, Star, UsersThree, CaretDown, X, CornersIn, CornersOut } from 'phosphor-react';
import { shareText } from '../nativeBridge';
import { toggleChannelMuted } from '../notifyChannels';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { shareLinkFor } from '../../utils/shareLink';
import { buildProviderUrl, isTwitchStream, streamProvider } from '../../utils/streamProvider';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useChannelSocial } from '../../hooks/useChannelSocial';
import { platformTerms } from '../../utils/platformTerms';
import { unwatchChannel, useChannelState, watchChannel } from '../../stores/channelStateStore';
import { useMobileHlsEngine, type MobilePlayerFailure } from './useMobileHlsEngine';
import { useVodProgressReporter } from '../../hooks/useVodProgressReporter';
import { listenSleepTimer, sleepTimerRunning, useSleepTimer } from './sleepTimer';
import { makeKey } from '../../utils/providerKey';
import { setLiveVideo, toggleLiveMute } from './liveVideo';
import { attachLockScreenAudio, releaseLockScreenAudio, updateNowPlaying } from './lockScreenAudio';
import { QualitySheet } from './QualitySheet';
import PenroseMarch from '../../components/PenroseMarch';
import { CollabAvatarStack } from '../../components/SharedViewers';
import { collabLabel, isTogether } from '../../utils/sharedViewers';
import { TogetherSheet } from './TogetherSheet';
import { CastSheet } from '../cast/CastSheet';
import { isCastable, useCastState } from '../cast/castState';
import { MobileOfflineRoom } from './MobileOfflineRoom';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';
import { registerReplayClock } from '../../stores/vodReplayStore';
import { VodSeekBar, VodTransport, useVodClock } from './VodControls';
import { recordedLine, seekTarget } from '../watch/channelMedia';
import { useChannelSheet } from '../watch/MobileChannelSheet';

const CONTROLS_HIDE_MS = 3000;
// Every bottom-bar glyph at one size, so no button out-pulls another.
const BAR_ICON = 20;

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

// The error line says what went wrong in words a viewer can act on, rather
// than one "Stream unavailable" for every cause.
function failureLine(reason: MobilePlayerFailure | null): string {
  switch (reason) {
    case 'network':
      return 'The stream stopped loading. Check your connection.';
    case 'stall':
      return 'The stream stopped sending video.';
    case 'timeout':
      return 'The stream took too long to start.';
    case 'media':
      return 'This phone could not play the video.';
    case 'unsupported':
      return 'This phone cannot play live video here.';
    default:
      return 'Playback failed. Try again in a moment.';
  }
}

// The header gives way in this order when the row runs out of room, and never
// pushes anything onto a second line: the Together chip keeps only its faces,
// then the uptime goes, then Subscribe drops out whole (a pill cut to "Subsc"
// says less than none). Past that the name truncates, last of all.
const GIVE_WAY_FACES = 1;
const GIVE_WAY_UPTIME = 2;
const GIVE_WAY_SUBSCRIBE = 3;
// Gaps the row puts between its parts, matching the classes below: the name
// block to Subscribe (ml-2), and the minimum before the metrics (pl-2).
const GAP_SUB = 8;
const GAP_METRICS = 8;

/** The bottom bar's buttons, left to right: the most used at the right, under
 *  the thumb, Minimize last. Clip joins after Share once the phone clips land
 *  (the iPhone has it there already). */
const BAR_ORDER = ['cast', 'share', 'mute', 'reload', 'settings', 'chat', 'rotate', 'minimize'] as const;

export const MobilePlayer: React.FC<{
  /** Landscape immersive mode: overlay carries no bottom rounding, adds insets. */
  immersive?: boolean;
  onToggleFullscreen?: () => void;
  /** Shrink into the floating player. In-app that is the mini box; leaving the
   *  app hands the same thing to the OS as system picture-in-picture. */
  onMinimize?: () => void;
  /** Turns the screen between portrait and landscape full screen. */
  onRotate?: () => void;
  /** The screen is on its side now (the rotate button's glyph and label). */
  landscapeNow?: boolean;
  /** Mini/PiP presentation: video only, no overlay chrome. */
  compact?: boolean;
  /** Current big-screen arrangement, for the toggle's icon and label. */
  layoutMode?: 'columns' | 'stacked';
  /** Set only where both arrangements fit, which is what gates the control. */
  onToggleLayout?: () => void;
  /** Whether landscape chat is showing, for the toggle's icon and label. */
  chatOpen?: boolean;
  /** Keep the control layer off one edge: landscape chat floats there. */
  controlsInset?: { side: 'left' | 'right'; px: number };
  /** An offline room's card sits in a chin below the band (MobileOfflineChin),
   *  so the band draws the offline art alone. */
  offlineCardBelow?: boolean;
}> = ({
  immersive = false,
  onToggleFullscreen,
  onMinimize,
  onRotate,
  landscapeNow = false,
  compact = false,
  layoutMode,
  onToggleLayout,
  chatOpen = false,
  controlsInset,
  offlineCardBelow = false,
}) => {
  const videoRef = useRef<HTMLVideoElement>(null);
  const { state, reason } = useMobileHlsEngine(videoRef);
  // The core's sleep timer: its fade and its stop reach the player from here,
  // and a quiet moon in the header says one is running.
  useEffect(() => listenSleepTimer(), []);
  // A VOD's chat replay (Rust's, as on desktop and the iPhone) follows this
  // player's clock.
  useEffect(() => registerReplayClock(() => videoRef.current?.currentTime ?? null), []);
  const sleepTimer = useSleepTimer((s) => s.state);
  const sleepOn = sleepTimerRunning(sleepTimer);
  // VOD position checkpoints to Rust, as the desktop player sends them: Rust
  // decides from the start's play stamp what each report may write (only a VOD
  // the viewer opened files a position). Live, clips and idle attach nothing.
  const vodPlayback = useAppStore((s) => s.vodPlayback);
  const vodStream = useAppStore((s) => s.currentStream);
  useVodProgressReporter(
    videoRef,
    vodPlayback
      ? {
          videoId: vodPlayback.video_id,
          playId: vodPlayback.play_id,
          channelLogin: vodPlayback.channel_login ?? vodStream?.user_login,
          channelName: vodStream?.user_name,
          title: vodPlayback.title ?? vodStream?.title,
          thumbnailUrl: vodPlayback.thumbnail_url ?? vodStream?.thumbnail_url,
        }
      : null,
  );
  const restartStream = useAppStore((s) => s.restartStream);
  const currentStream = useAppStore((s) => s.currentStream);
  // A channel that is not streaming, opened as its offline room.
  const offline = useAppStore((s) => s.streamUrl === 'offline');
  // A VOD plays (a channel's video, or an offline room's latest broadcast): its
  // own header line and transport, and no mute or reload, as on the iPhone. A
  // live rewind (media type live) keeps the live chrome.
  const currentMediaType = useAppStore((s) => s.currentMediaType);
  const vod = vodPlayback && currentMediaType !== 'live' && !offline ? vodPlayback : null;
  const roomVod = !!vod && currentMediaType === 'offline_chat';
  const leaveRoomVod = useAppStore((s) => s.leaveOfflineRoomVod);
  const vodClock = useVodClock(videoRef, vod);
  // The VOD's speed, back to 1x for the next one. Set on the element and as its
  // default, so a reload of the same VOD keeps it.
  const [vodRate, setVodRate] = useState(1);
  const [rateFor, setRateFor] = useState<string | null>(null);
  if ((vod?.video_id ?? null) !== rateFor) {
    setRateFor(vod?.video_id ?? null);
    setVodRate(1);
  }
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !vod) return;
    const apply = () => {
      video.defaultPlaybackRate = vodRate;
      video.playbackRate = vodRate;
    };
    apply();
    video.addEventListener('loadedmetadata', apply);
    return () => {
      video.removeEventListener('loadedmetadata', apply);
      video.defaultPlaybackRate = 1;
      video.playbackRate = 1;
    };
  }, [vod, vodRate]);
  const seekVod = (seconds: number) => {
    const video = videoRef.current;
    if (video) video.currentTime = seekTarget(seconds, vodClock.length);
  };
  // A provider stream that ended or could not start keeps its chat, and the
  // band says which (AppStore's playbackNotice). Read only for the stream it
  // was written for.
  const playbackNotice = useAppStore((s) => s.playbackNotice);
  const noticeLine =
    offline && currentStream && playbackNotice?.key === makeKey(streamProvider(currentStream), currentStream.user_login)
      ? playbackNotice.message
      : null;
  // Channel state, avatar repair and live alerts are Twitch services keyed by
  // login or numeric id; a Kick slug or id would address an unrelated Twitch
  // channel of the same name.
  const streamIsTwitch = isTwitchStream(currentStream);
  const streamProv = streamProvider(currentStream);
  // Subscribe sits beside who it is, like the live-alert bell. The shared hook
  // opens the platform's own checkout in the login overlay and reports whether
  // you already subscribe, so the pill can offer a gift instead.
  const canSubscribe = !!currentStream && (streamProv === 'twitch' || streamProv === 'kick');
  const sub = useChannelSocial({
    provider: streamProv,
    userId: streamIsTwitch ? (currentStream?.user_id ?? null) : null,
    userLogin: currentStream?.user_login ?? null,
    userName: currentStream?.user_name ?? null,
    enabled: canSubscribe,
  });
  const subTerms = platformTerms(streamProv);

  // Live viewer count from the Rust channel_state service (one Helix batch
  // for every watched channel, emitted only on change), the same source the
  // desktop chat header uses. Without it the count was whatever startStream
  // captured and never moved.
  const channelStateLogin = streamIsTwitch ? (currentStream?.user_login?.toLowerCase() ?? null) : null;
  const channelState = useChannelState(channelStateLogin);
  useEffect(() => {
    if (!channelStateLogin || !currentStream?.user_id) return;
    void watchChannel(channelStateLogin, currentStream.user_id);
    return () => {
      void unwatchChannel(channelStateLogin);
    };
  }, [channelStateLogin, currentStream?.user_id]);
  // Shared Viewership: a Together chip beside the channel's own count, tapped
  // open into who else is on. Outside a group, the Twitch Shared Chat session
  // takes the same chip ("Shared chat" and the partners' faces).
  const collab = channelState?.collab ?? channelState?.shared_chat ?? null;
  const viewerCount = channelState?.viewer_count ?? currentStream?.viewer_count ?? 0;

  // Title, category and start time follow the same pushes, so a retitle or a
  // game change shows here without reopening the stream. A push that left a
  // field out never blanks it: the last value a push carried holds, and the
  // row the stream opened with stands in until the first one.
  const pushTitle = channelState?.title || null;
  const pushGame = channelState?.game_name || null;
  const pushStarted = channelState?.started_at || null;
  const [liveFacts, setLiveFacts] = useState<{
    login: string | null;
    title: string | null;
    game: string | null;
    started: string | null;
  }>({ login: null, title: null, game: null, started: null });
  // Render-time adjustment, as the restart phase below does: no effect round
  // trip, and the guards make it settle in one pass.
  if (liveFacts.login !== channelStateLogin) {
    setLiveFacts({ login: channelStateLogin, title: pushTitle, game: pushGame, started: pushStarted });
  } else if (
    (pushTitle && pushTitle !== liveFacts.title) ||
    (pushGame && pushGame !== liveFacts.game) ||
    (pushStarted && pushStarted !== liveFacts.started)
  ) {
    setLiveFacts({
      login: channelStateLogin,
      title: pushTitle ?? liveFacts.title,
      game: pushGame ?? liveFacts.game,
      started: pushStarted ?? liveFacts.started,
    });
  }
  const liveTitle = liveFacts.title || currentStream?.title || '';
  const liveCategory = liveFacts.game || currentStream?.game_name || '';
  const liveStartedAt = liveFacts.started || currentStream?.started_at;

  // A stream that arrived without its avatar (a lookup hiccup on open) gets
  // one repair, keyed so a second miss for the same channel does not loop.
  const healedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!streamIsTwitch || !currentStream?.user_id || currentStream.profile_image_url?.trim()) return;
    if (healedFor.current === currentStream.user_id) return;
    healedFor.current = currentStream.user_id;
    const id = currentStream.user_id;
    void invoke<{ profile_image_url?: string; broadcaster_type?: string }>('get_user_by_id', { userId: id })
      .then((u) => {
        const cs = useAppStore.getState().currentStream;
        if (!cs || cs.user_id !== id || !u?.profile_image_url) return;
        useAppStore.setState({
          currentStream: {
            ...cs,
            profile_image_url: u.profile_image_url,
            broadcaster_type: cs.broadcaster_type ?? u.broadcaster_type,
          },
        });
      })
      .catch(() => {});
  }, [streamIsTwitch, currentStream?.user_id, currentStream?.profile_image_url]);

  // Subscribed rather than read through the helper, so toggling the bell
  // re-renders this overlay instead of leaving a stale icon behind.
  const mutedChannels = useAppStore((s) => s.settings.live_notifications?.muted_live_channels);
  const alertsOff = !!currentStream && (mutedChannels ?? []).includes(currentStream.user_login.toLowerCase());

  const [controlsVisible, setControlsVisible] = useState(true);
  const [paused, setPaused] = useState(false);
  const [muted, setMuted] = useState(false);
  const [qualityOpen, setQualityOpen] = useState(false);
  const [togetherOpen, setTogetherOpen] = useState(false);
  const [castOpen, setCastOpen] = useState(false);
  const castOffered = useCastState().offered;
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
  const nowPlaying = currentStream
    ? {
        title: liveTitle || currentStream.user_name,
        artist: currentStream.user_name,
        artUrl: currentStream.profile_image_url ?? '',
      }
    : null;
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !nowPlaying) return;
    return attachLockScreenAudio(video, nowPlaying);
    // Keyed on the CHANNEL only. A title or avatar change is a card repaint,
    // handled below; re-running this effect would tear the session down.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentStream?.user_login]);
  useEffect(() => {
    if (nowPlaying) updateNowPlaying(nowPlaying);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nowPlaying?.title, nowPlaying?.artist, nowPlaying?.artUrl]);

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

  // A stall that lasts reads as a frozen frame unless something says it is
  // loading. Brief rebuffers stay quiet; one past a second shows the mark.
  const [stallShown, setStallShown] = useState(false);
  if (state !== 'stalled' && stallShown) setStallShown(false);
  useEffect(() => {
    if (state !== 'stalled') return;
    const t = setTimeout(() => setStallShown(true), 1000);
    return () => clearTimeout(t);
  }, [state]);

  const logoWanted = state === 'loading' || state === 'idle' || stallShown;
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

  // Back to full size from the mini box (or picture in picture), the controls
  // show for their usual spell, as on the iPhone. The watch layer takes every
  // press until it has landed, so they cannot be pressed mid-flight.
  const [prevCompact, setPrevCompact] = useState(compact);
  if (prevCompact !== compact) {
    setPrevCompact(compact);
    if (!compact) setControlsVisible(true);
  }
  useEffect(() => {
    if (!compact) scheduleHide();
  }, [compact, scheduleHide]);

  // Mute from the system PiP window. Web content cannot draw usable controls
  // there, so the toggle is a native RemoteAction; MainActivity broadcasts back
  // into the WebView, and the shared toggle pushes the resulting state out
  // again so the OS can flip its icon between mute and unmute.
  useEffect(() => {
    const onPipMute = () => {
      const next = toggleLiveMute();
      if (next !== null) setMuted(next);
    };
    window.addEventListener('sn:pip-mute', onPipMute);
    return () => window.removeEventListener('sn:pip-mute', onPipMute);
  }, []);

  // The buttons follow the element, not the last tap: a headset, the lock
  // screen, a call or the engine's muted-autoplay fallback all pause or mute
  // the video without going through these controls.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const onPlay = () => setPaused(false);
    const onPause = () => setPaused(true);
    const onVolume = () => setMuted(video.muted);
    video.addEventListener('play', onPlay);
    video.addEventListener('pause', onPause);
    video.addEventListener('volumechange', onVolume);
    return () => {
      video.removeEventListener('play', onPlay);
      video.removeEventListener('pause', onPause);
      video.removeEventListener('volumechange', onVolume);
    };
  }, []);

  // Stable, so React attaches it once per element rather than detaching and
  // re-attaching on every render: the published element also carries the
  // play/pause state the watch layer's keep-awake reads, which a per-render
  // detach would reset.
  const attachVideo = useCallback((el: HTMLVideoElement | null) => {
    (videoRef as React.MutableRefObject<HTMLVideoElement | null>).current = el;
    setLiveVideo(el);
  }, []);

  // Header give-way (see GIVE_WAY_*), decided from hidden copies of each part
  // at full size, so the choice never depends on what is already cut.
  const identityRowRef = useRef<HTMLDivElement>(null);
  const measureNameRef = useRef<HTMLSpanElement>(null);
  const measureSubRef = useRef<HTMLSpanElement>(null);
  const measureFullRef = useRef<HTMLSpanElement>(null);
  const measureFacesRef = useRef<HTMLSpanElement>(null);
  const measureNoUptimeRef = useRef<HTMLSpanElement>(null);
  const [giveWay, setGiveWay] = useState(0);
  const uptimeText = formatUptime(liveStartedAt, nowMs);
  const subLabel = sub.isSubscribed ? subTerms.paidGift : sub.hasSubHistory ? subTerms.paidAgain : subTerms.paid;
  useLayoutEffect(() => {
    const row = identityRowRef.current;
    if (!row) return;
    const width = (el: Element | null) => (el ? el.getBoundingClientRect().width : 0);
    const measure = () => {
      const available = row.clientWidth;
      const name = width(measureNameRef.current);
      const subscribe = canSubscribe ? width(measureSubRef.current) + GAP_SUB : 0;
      const metrics = [
        width(measureFullRef.current),
        width(measureFacesRef.current),
        width(measureNoUptimeRef.current),
      ];
      const needs = (level: number) =>
        name + (level >= GIVE_WAY_SUBSCRIBE ? 0 : subscribe) + GAP_METRICS + metrics[Math.min(level, GIVE_WAY_UPTIME)];
      let level = 0;
      while (level < GIVE_WAY_SUBSCRIBE && needs(level) > available + 0.5) level += 1;
      setGiveWay(level);
    };
    measure();
    // The row follows the viewport and the landscape chat inset; the copies
    // follow late loads (a subscriber badge) and live numbers.
    const observer = new ResizeObserver(measure);
    for (const el of [
      row,
      measureNameRef.current,
      measureSubRef.current,
      measureFullRef.current,
      measureFacesRef.current,
      measureNoUptimeRef.current,
    ]) {
      if (el) observer.observe(el);
    }
    return () => observer.disconnect();
  }, [canSubscribe, currentStream?.user_name, currentStream?.broadcaster_type, streamIsTwitch, subLabel, collab, uptimeText, viewerCount, sleepOn]);

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
    // The shared toggle also keeps the PiP action's icon in step.
    const next = toggleLiveMute();
    if (next === null) return;
    setMuted(next);
    scheduleHide();
  };

  // The header's parts, drawn once for real and once as the hidden full-size
  // copies the give-way measures (`interactive` false: the same boxes, no
  // buttons).
  const nameBlock = (interactive: boolean) =>
    currentStream && (
      <>
        <span className="text-[14px] leading-5 font-semibold text-white truncate">{currentStream.user_name}</span>
        {currentStream.broadcaster_type === 'partner' && (
          <svg className="w-3 h-3 shrink-0" viewBox="0 0 16 16" fill="#9146FF" aria-hidden>
            <path
              fillRule="evenodd"
              d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z"
              clipRule="evenodd"
            ></path>
          </svg>
        )}
        {/* Live-alert opt-out for this channel, next to who it is rather than
            down in the playback controls: it is a fact about the channel, not
            about this session's playback. Quiet, a step below the name. */}
        {streamIsTwitch &&
          (interactive ? (
            <button
              onClick={(e) => {
                e.stopPropagation();
                void toggleChannelMuted(currentStream.user_login);
              }}
              aria-label={
                alertsOff
                  ? `Turn on live alerts for ${currentStream.user_name}`
                  : `Turn off live alerts for ${currentStream.user_name}`
              }
              aria-pressed={!alertsOff}
              className={`relative shrink-0 w-3.5 h-5 flex items-center justify-center transition-colors ${
                alertsOff ? 'text-white/45' : 'text-white/70'
              }`}
            >
              {alertsOff ? <BellSlash size={11} weight="bold" /> : <Bell size={11} weight="fill" />}
              {/* A finger-sized target around the small glyph; the row keeps
                  its spacing. */}
              <span className="absolute -inset-2.5" />
            </button>
          ) : (
            <span className="shrink-0 w-3.5 h-5" />
          ))}
      </>
    );

  // A step below the name it sits beside: secondary ink at ordinary weight.
  const subscribeClass =
    'shrink-0 flex items-center gap-1 pl-1.5 pr-2 py-[3px] rounded-full bg-white/15 text-[12px] font-medium text-white/75 leading-none whitespace-nowrap';
  const subscribeInk = (
    <>
      {sub.subscriberBadgeUrl ? (
        <img src={sub.subscriberBadgeUrl} alt="" draggable={false} className="w-3.5 h-3.5" />
      ) : (
        <Star size={12} weight="fill" />
      )}
      {subLabel}
    </>
  );
  // Subscribe sits beside who it is. Whole or not at all: GIVE_WAY_SUBSCRIBE
  // drops it before the name gives way, never cut to a fragment.
  const subscribePill = (interactive: boolean) =>
    interactive ? (
      <button
        onClick={(e) => {
          e.stopPropagation();
          void sub.handleSubscribeClick();
        }}
        aria-label={`${subLabel}, ${currentStream?.user_name ?? ''}`}
        className={`${subscribeClass} active:scale-95 transition-transform`}
      >
        {subscribeInk}
      </button>
    ) : (
      <span className={subscribeClass}>{subscribeInk}</span>
    );

  // The right side: the group, viewers, and the uptime (only when the start is
  // known). White at 80 percent, not live red: the numbers inform, the name
  // leads.
  const metrics = (compactGroup: boolean, withUptime: boolean, interactive: boolean) => {
    const togetherClass = `flex items-center gap-1 rounded-full bg-white/15 py-0.5 pl-0.5 ${
      compactGroup ? 'pr-0.5' : 'pr-2'
    } text-[11.5px] font-semibold text-white/85 leading-none`;
    const together = collab && (
      <>
        <CollabAvatarStack collab={collab} size={16} ringClass="ring-black/60" />
        {!compactGroup && (isTogether(collab) ? 'Together' : 'Shared chat')}
      </>
    );
    return (
      <span className="flex items-center gap-2.5 text-[12px] font-medium leading-none text-white/80 tabular-nums whitespace-nowrap">
        {collab &&
          (interactive ? (
            <button
              onClick={(e) => {
                e.stopPropagation();
                setTogetherOpen(true);
              }}
              aria-label={collabLabel(collab)}
              className={`${togetherClass} active:scale-95 transition-transform`}
            >
              {together}
            </button>
          ) : (
            <span className={togetherClass}>{together}</span>
          ))}
        <span className="flex items-center gap-1">
          <UsersThree size={11} weight="fill" aria-hidden />
          {viewerCount.toLocaleString()}
          <span className="sr-only"> viewers</span>
        </span>
        {withUptime && uptimeText && <span>{uptimeText}</span>}
        {sleepOn && (
          <MoonStars size={11} weight="fill" aria-label={sleepTimer?.rowLabel ?? 'Sleep timer on'} />
        )}
      </span>
    );
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
        ref={attachVideo}
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
          // Sized by the band it sits in, not by a fixed pixel count: a
          // phone's 16:9 band is ~200px tall and an 84px mark was half of it.
          // `.sn-load-mark` reads the band's height through a container
          // query (this overlay is inset-0, so its size IS the band's) and
          // clamps for mini/PiP at the bottom and tablets at the top.
          style={{ containerType: 'size' }}
        >
          <PenroseMarch className="sn-load-mark" />
        </div>
      )}

      {/* An offline room: who, when, what is next, and the stream starts here
          when they go live. Not an error, so no Retry and no live chrome. The
          mini box shows it compact: the face and an Offline ring. */}
      {offline && !noticeLine && <MobileOfflineRoom mini={compact} cardBelow={offlineCardBelow} />}

      {/* The room has no player controls, so its way out is always on show:
          minimize (back to the page it was opened from, the room in the mini
          box) and close, in the band's top corner. */}
      {offline && !compact && (
        <div
          className="absolute right-1 z-20 flex"
          style={{ top: immersive ? 'calc(var(--sn-safe-t, 0px) + 4px)' : 4 }}
        >
          {onMinimize && (
            <button
              onClick={(e) => {
                e.stopPropagation();
                onMinimize();
              }}
              className="sn-touch flex items-center justify-center"
              aria-label="Minimize"
            >
              <span className="flex h-8 w-8 items-center justify-center rounded-full bg-black/60 text-white">
                <CaretDown size={15} weight="bold" />
              </span>
            </button>
          )}
          <button
            onClick={(e) => {
              e.stopPropagation();
              void useAppStore.getState().exitStream();
            }}
            className="sn-touch flex items-center justify-center"
            aria-label="Close"
          >
            <span className="flex h-8 w-8 items-center justify-center rounded-full bg-black/60 text-white">
              <X size={13} weight="bold" />
            </span>
          </button>
        </div>
      )}

      {/* Playback that cannot go on: what happened in plain words, and Retry.
          A provider stream that ended or never started says so here while its
          chat keeps running below. Not in the mini box, which has no room for
          a sentence and whose taps belong to the box. */}
      {!compact && (noticeLine || (state === 'error' && !offline)) && (
        <div
          className="absolute inset-0 flex flex-col items-center justify-center gap-3.5 bg-black/70 p-6"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="text-[14px] text-white/80 text-center">{noticeLine ?? failureLine(reason)}</div>
          <button
            onClick={(e) => {
              e.stopPropagation();
              void restartStream();
            }}
            className="chrome-glaze chrome-glaze--control chrome-glaze--frosted px-5 py-2.5 text-sm font-semibold text-white"
          >
            Retry
          </button>
        </div>
      )}

      {/* Control overlay. Carries the persistent stream info (channel, title,
          category, viewers, uptime) so nothing has to sit between the player
          and chat, matching how the Twitch app does it. */}
      <div
        // No fade into the mini box: the controls go the moment a minimize
        // starts (as Xtra hides its controller), rather than fading out while
        // the player shrinks under them.
        className={`absolute inset-0 ${compact ? '' : 'transition-opacity duration-200'} ${
          controlsVisible && !compact && !offline ? 'opacity-100' : 'opacity-0 pointer-events-none'
        }`}
        style={{
          // Dark at the top and the bottom, clear through the middle. The top
          // reaches past the header's category line, so all three lines stay
          // readable over bright video.
          background:
            'linear-gradient(to bottom, rgba(0,0,0,0.66) 0%, rgba(0,0,0,0.45) 30%, transparent 46%, transparent 60%, rgba(0,0,0,0.35) 78%, rgba(0,0,0,0.7) 100%)',
          ...(controlsInset ? { [controlsInset.side]: controlsInset.px } : null),
        }}
      >
        {/* Stream info, top-left. The avatar anchors a column: the name, the
            title and the category all start at the name's edge, so the three
            read as one block about one stream. The live red is the avatar's
            ring alone; the counts stay white so they never out-pull the name. */}
        {/* A scrim under the header alone, plus a soft shadow on its text, so
            the category line holds on bright video. */}
        {currentStream && (
          <div
            aria-hidden
            className="absolute inset-x-0 top-0 pointer-events-none"
            style={{
              height: immersive ? 'calc(var(--sn-safe-t, 0px) + 72px)' : 72,
              background: 'linear-gradient(to bottom, rgba(0,0,0,0.6) 0%, rgba(0,0,0,0.3) 60%, transparent 100%)',
            }}
          />
        )}
        {currentStream && (
          <div
            className="absolute inset-x-0 top-0 px-3 pt-2 [text-shadow:0_1px_2px_rgba(0,0,0,0.6)]"
            style={immersive ? { paddingTop: 'calc(var(--sn-safe-t, 0px) + 8px)' } : undefined}
          >
            <div className="flex items-start gap-2.5">
              {/* The face opens the channel's videos and clips (Twitch's), as on
                  the iPhone. The live red ring is for live alone. */}
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  if (streamIsTwitch && currentStream.user_id) useChannelSheet.getState().open(currentStream);
                }}
                disabled={!streamIsTwitch || !currentStream.user_id}
                aria-label={`${currentStream.user_name}'s videos and clips`}
                className="shrink-0 rounded-full"
              >
                {currentStream.profile_image_url ? (
                  <SevenTvAvatar
                    cosmeticsId={sevenTvIdForStream(currentStream)}
                    px={32}
                    motion="animate"
                    src={currentStream.profile_image_url}
                    alt=""
                    draggable={false}
                    className={`w-8 h-8 rounded-full object-cover shrink-0 ring-2 ${vod ? 'ring-white/25' : 'ring-live/80'}`}
                  />
                ) : (
                  <div
                    className={`w-8 h-8 rounded-full bg-white/15 shrink-0 flex items-center justify-center text-[12px] font-bold text-white ring-2 ${
                      vod ? 'ring-white/25' : 'ring-live/80'
                    }`}
                  >
                    {currentStream.user_name?.charAt(0).toUpperCase()}
                  </div>
                )}
              </button>
              <div className="flex-1 min-w-0 flex flex-col gap-0.5">
                {/* Identity row: the name, then Subscribe, then the live
                    numbers opposite. One line always; parts give way in order
                    (GIVE_WAY_*) instead of wrapping. */}
                <div ref={identityRowRef} className="relative flex items-center min-w-0 h-5">
                  <span className="flex items-center gap-1 min-w-0">{nameBlock(true)}</span>
                  {canSubscribe && giveWay < GIVE_WAY_SUBSCRIBE && (
                    <span className="ml-2 shrink-0 flex">{subscribePill(true)}</span>
                  )}
                  {!vod && (
                    <span className="ml-auto pl-2 shrink-0 flex">
                      {metrics(giveWay >= GIVE_WAY_FACES, giveWay < GIVE_WAY_UPTIME, true)}
                    </span>
                  )}
                  {/* Full-size copies for the give-way measurement. Invisible,
                      out of the accessibility tree, and in a zero-size clip so
                      their width never adds overflow to the row; clipped boxes
                      still measure at their natural size. */}
                  <div
                    aria-hidden
                    className="absolute left-0 top-0 w-0 h-0 overflow-hidden invisible pointer-events-none"
                  >
                    <div className="w-max flex items-center gap-4">
                      <span ref={measureNameRef} className="flex items-center gap-1">
                        {nameBlock(false)}
                      </span>
                      {canSubscribe && (
                        <span ref={measureSubRef} className="flex">
                          {subscribePill(false)}
                        </span>
                      )}
                      <span ref={measureFullRef} className="flex">
                        {metrics(false, true, false)}
                      </span>
                      <span ref={measureFacesRef} className="flex">
                        {metrics(true, true, false)}
                      </span>
                      <span ref={measureNoUptimeRef} className="flex">
                        {metrics(true, false, false)}
                      </span>
                    </div>
                  </div>
                </div>
                {vod ? (
                  <>
                    {(vod.title || currentStream.title) && (
                      <div className="text-[13px] leading-[17px] text-white/85 truncate">
                        {vod.title || currentStream.title}
                      </div>
                    )}
                    <div className="text-[12px] leading-4 text-white/70 truncate">{recordedLine(vod.recorded_at)}</div>
                  </>
                ) : (
                  <>
                    {liveTitle && (
                      <div className="text-[13px] leading-[17px] text-white/85 truncate">{liveTitle}</div>
                    )}
                    {liveCategory && (
                      <div className="text-[12px] leading-4 text-white/70 truncate">{liveCategory}</div>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>
        )}
        {/* Center play/pause */}
        <button
          onClick={togglePlay}
          className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-16 h-16 rounded-full bg-glass-ink/45 flex items-center justify-center text-white active:scale-95 transition-transform"
          aria-label={paused ? 'Play' : 'Pause'}
        >
          {/* A triangle's visual weight sits left of its box center; the
              nudge puts it where the eye expects the middle. */}
          {paused ? (
            <Play size={30} weight="fill" style={{ transform: 'translateX(7%)' }} />
          ) : (
            <Pause size={30} weight="fill" />
          )}
        </button>

        {/* Bottom bar, on the header's visible edges: the LIVE dot starts at
            the avatar ring's outer edge (pl-2.5: px-3 less the 2px ring), and
            the last icon's ink ends where the uptime text does. */}
        <div
          className="absolute inset-x-0 bottom-0 flex flex-col pl-2.5 pr-3 pb-2"
          style={immersive ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 8px)' } : undefined}
        >
          {vod && (
            <VodSeekBar
              vod={vod}
              position={vodClock.position}
              length={vodClock.length}
              onSeek={seekVod}
              onTouch={scheduleHide}
            />
          )}
          <div className="flex items-center justify-between">
          {vod ? (
            // Where LIVE sits on a live stream: the way back to the room when its
            // latest broadcast plays, then back and forward ten seconds and the speed.
            <div className="-ml-3 flex shrink-0 items-center">
              {roomVod && (
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    void leaveRoomVod();
                  }}
                  className="flex h-11 items-center gap-1 pl-3 pr-1.5 text-[12px] font-bold text-white"
                  aria-label="Back to the room"
                >
                  <CaretLeft size={13} weight="bold" />
                  Room
                </button>
              )}
              <VodTransport
                rate={vodRate}
                onSkip={(by) => seekVod((videoRef.current?.currentTime ?? vodClock.position) + by)}
                onRate={setVodRate}
                onTouch={scheduleHide}
                iconSize={BAR_ICON}
              />
            </div>
          ) : (
            /* A status, not a control: a dot and the word, no filled badge.
               Gone while playback has failed, which is not live. */
            <div
              className={`flex shrink-0 items-center gap-[5px] ${state === 'error' ? 'opacity-0' : ''}`}
              role="img"
              aria-label="Live"
              aria-hidden={state === 'error'}
            >
              <span
                className="w-[7px] h-[7px] rounded-full bg-live shrink-0"
                style={{ boxShadow: '0 0 3px color-mix(in srgb, var(--color-live) 60%, transparent)' }}
              />
              <span className="text-[12px] font-bold tracking-[0.4px] text-white leading-none">LIVE</span>
            </div>
          )}
          {/* One size and one weight for every bar icon, so no button
              out-pulls another. The last icon's ink, not its 44 px target,
              ends on the header's right edge: 12px of target padding plus the
              glyph's own inset sit past the row's padding. */}
          {/* The bar's buttons, left to right: the most used at the right,
              under the thumb, Minimize last. One list, so a change of order is
              one line (iPhone keeps the same list in PlayerChrome). */}
          <div className="flex min-w-0 items-center -mr-[13px]">
            {BAR_ORDER.map((slot) => (
              <React.Fragment key={slot}>
                {slot === 'cast' && (
                  <>
                  {/* Cast and mirroring, together: the two ways onto a TV. Only on
                      builds that offer them (Rust cast_session::OFFERED). */}
                  {castOffered && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setCastOpen(true);
                      }}
                      className="sn-bar-btn flex items-center justify-center text-white"
                      aria-label="Watch on a TV"
                    >
                      <Screencast size={BAR_ICON} />
                    </button>
                  )}
                  </>
                )}
                {slot === 'share' && (
                  <>
                  {currentStream && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        // Our own share link on every platform, not a raw platform
                        // URL: the landing page hands off to StreamNook when it is
                        // installed and falls back to the platform's page when it is
                        // not. Same Rust-built link the desktop share copies.
                        const stream = currentStream;
                        void shareLinkFor(stream)
                          .catch(() => buildProviderUrl(streamProvider(stream), stream.user_login))
                          .then((link) =>
                            shareText(`${stream.user_name} is live: ${link}`, liveTitle || stream.title || stream.user_name),
                          );
                      }}
                      className="sn-bar-btn flex items-center justify-center text-white"
                      aria-label="Share stream"
                    >
                      <ShareNetwork size={BAR_ICON} />
                    </button>
                  )}
                  </>
                )}
                {slot === 'mute' && !vod && (
                  <>
                  <button
                    onClick={toggleMute}
                    className="sn-bar-btn flex items-center justify-center text-white"
                    aria-label={muted ? 'Unmute' : 'Mute'}
                  >
                    {muted ? <SpeakerSlash size={BAR_ICON} /> : <SpeakerHigh size={BAR_ICON} />}
                  </button>
                  </>
                )}
                {slot === 'reload' && !vod && (
                  <>
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
                    className="sn-bar-btn flex items-center justify-center text-white"
                    aria-label="Restart stream"
                  >
                    <ArrowClockwise size={BAR_ICON} className={restarting ? 'animate-spin' : ''} />
                  </button>
                  </>
                )}
                {slot === 'settings' && (
                  <>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setQualityOpen(true);
                    }}
                    className="sn-bar-btn flex items-center justify-center text-white"
                    aria-label="Quality"
                  >
                    <Gear size={BAR_ICON} />
                  </button>
                  {/* Only on a screen where both arrangements genuinely work, so this
                      never appears as a control with one sensible setting. */}
                  {onToggleLayout && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onToggleLayout();
                      }}
                      className="sn-bar-btn flex items-center justify-center text-white"
                      aria-label={
                        layoutMode === 'columns' ? 'Stack player above chat' : 'Put chat beside player'
                      }
                    >
                      {layoutMode === 'columns' ? <Rows size={BAR_ICON} /> : <Columns size={BAR_ICON} />}
                    </button>
                  )}
                  </>
                )}
                {slot === 'chat' && (
                  <>
                  {onToggleFullscreen && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onToggleFullscreen();
                      }}
                      className="sn-bar-btn flex items-center justify-center text-white"
                      aria-label={chatOpen ? 'Hide chat' : 'Show chat'}
                    >
                      {/* One glyph, filled while chat is showing: this button is
                          "chat" in both states, and a crossed-out icon read as a
                          warning rather than as the way back. */}
                      <ChatTeardropText size={BAR_ICON} weight={chatOpen ? 'fill' : 'regular'} />
                    </button>
                  )}
                  </>
                )}
                {slot === 'rotate' && (
                  <>
                  {onRotate && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onRotate();
                        scheduleHide();
                      }}
                      className="sn-bar-btn flex items-center justify-center text-white"
                      aria-label={landscapeNow ? 'Rotate to portrait' : 'Rotate to landscape'}
                    >
                      {landscapeNow ? <CornersIn size={BAR_ICON} /> : <CornersOut size={BAR_ICON} />}
                    </button>
                  )}
                  </>
                )}
                {slot === 'minimize' && (
                  <>
                  {onMinimize && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onMinimize();
                      }}
                      className="sn-bar-btn flex items-center justify-center text-white"
                      aria-label="Minimize player"
                    >
                      <CaretDown size={BAR_ICON} />
                    </button>
                  )}
                  </>
                )}
              </React.Fragment>
            ))}
          </div>
          </div>
        </div>
      </div>

      <QualitySheet open={qualityOpen} onClose={() => setQualityOpen(false)} />
      <TogetherSheet collab={collab} open={togetherOpen} onClose={() => setTogetherOpen(false)} />
      {castOffered && (
        <CastSheet open={castOpen} onClose={() => setCastOpen(false)} castable={isCastable(currentStream)} />
      )}
    </div>
  );
};
