import { useRef, useEffect, useCallback, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { invoke } from '@tauri-apps/api/core';
import Hls from 'hls.js';
import Plyr from 'plyr';
// Plyr's CSS ships once, layered, via globals.css (@import ... layer(vendor));
// a second unlayered copy here would beat the app's control-bar overrides.
import { motion, AnimatePresence } from 'framer-motion';
import { Loader2, RefreshCcw, Home, LayoutGrid, Shield, ShieldCheck, ShieldAlert, Clapperboard, Music, Share2, Check, Radio } from 'lucide-react';
import { Heart, HeartBreak, ArrowLeft, X as XIcon } from 'phosphor-react';
import { useAppStore } from '../stores/AppStore';
import { streamProvider } from '../utils/streamProvider';
import { makeKey } from '../utils/providerKey';
import { canGridProvider, gridRefusal } from '../types/providers';
import { platformTerms } from '../utils/platformTerms';
import { useContextMenuStore } from '../stores/contextMenuStore';
import { buildShareUrl } from '../utils/shareLink';
import { playerOverlayButtonOn } from '../utils/playerOverlayButtons';
import { usemultiNookStore } from '../stores/multiNookStore';
import { useChannelSocial } from '../hooks/useChannelSocial';
import StreamTitleWithEmojis from './StreamTitleWithEmojis';
import PlayerStatsOverlay from './PlayerStatsOverlay';
import { useVodProgressReporter } from '../hooks/useVodProgressReporter';
import { formatVodTime } from '../utils/vodProgress';
import { createLiveEdgeTracker } from '../utils/liveEdge';
import BroadcastTimeline from './BroadcastTimeline';
import { Tooltip } from './ui/Tooltip';
import { TwitchVerifiedMark } from './ui/TwitchGlyph';
import { registerPlayerControls, type PlayerControls } from '../keybindings';
import { qualitiesEquivalent } from '../utils/quality';

import { Logger } from '../utils/logger';
import { syncTauriWindowFullscreen } from '../utils/windowFullscreen';
import { startLatencyGovernor, DEFAULT_LATENCY_BAND } from '../utils/liveLatencyGovernor';
import { LL_DISPLAY_CALIBRATION, LL_TARGET_DEFAULT } from '../utils/latency';
import { startLLDiagnostics, stopLLDiagnostics, llDiagNote, isLLDiagEnabled } from '../utils/llDiagnostics';
import {
  applyAudioBoost,
  resolveAudioBoost,
  audioBoostFaderDefs,
  audioBoostResetPatch,
} from '../utils/audioBoost';
import type { AudioBoostSettings } from '../types';
import { Fader, Toggle } from './AudioBoostFaders';

/** A backward scrub on a live stream smaller than this is treated as a
 *  nudge, not a rewind request. */
const LIVE_SCRUB_HANDOFF_SECS = 3;
/** Past this far behind the edge the whole-segment path is force-seeked
 *  forward by hls.js (liveMaxLatencyDuration), so a scrub that lands beyond
 *  it cannot be served live and hands off to the recording. */
const LIVE_DVR_MAX_BEHIND_SECS = 55;

import { open as openExternalUrl } from '@tauri-apps/plugin-shell';
import { setActiveVideo } from '../utils/activeVideo';
import { isWindowHidden } from '../utils/windowVisibility';
import { recognizeNowPlaying, announceSong } from '../utils/songId';
import {
  ignoresPlayerMouse,
  createWheelAccumulator,
  stepVolume,
  toggleVolumeMute,
  aboutRevealNeedsShift,
  scrollVolumeOn,
  WHEEL_VOLUME_STEP,
} from '../utils/playerMouseControls';
import { PlayerVolumeOsd } from './PlayerVolumeOsd';
import { useVolumeOsd } from '../hooks/useVolumeOsd';

// Paint the Audio Boost toggle that gets injected into Plyr's control bar so it
// reflects on/off. The `is-active` class lights it up as an accent chip (fill +
// inset rim, styled in globals.css, no outer glow); off falls back to the normal
// control. Module-level so the inject and sync effects share one copy.
function paintAudioBoostButton(btn: Element | null, on: boolean): void {
  if (!btn) return;
  btn.classList.toggle('is-active', on);
  btn.setAttribute('aria-pressed', on ? 'true' : 'false');
  const tip = btn.querySelector('.plyr__tooltip');
  if (tip) tip.textContent = on ? 'Audio Boost: On' : 'Audio Boost: Off';
}

// Per-channel learned cushion (LL path), relative to the viewer's chosen target. The
// `base` is the user's target latency converted to the real (raw) cushion. The
// stall-adaptive cushion proves a channel's delivery wobbliness during a session;
// remembering it means a wobbly channel pays its stalls ONCE ever instead of once per
// session, while clean channels keep the user's tight target. The learned value relaxes
// by 0.5s per day, so one bad night doesn't tax a channel forever, and it is clamped to
// [base, base + 2] so it never drops below the user's target nor runs away above it.
// Version the store: when a stall-causing pipeline bug is fixed, cushions learned from
// those stalls are poisoned lessons; bump to orphan and re-learn.
// Namespaced by PLATFORM: a Kick "xqc" and a Twitch "xqc" are different streams
// with different segment lengths, and a shared key would hand one the other's
// learned cushion.
const llCushionKey = (channel: string, provider = 'twitch') =>
  `streamnook.ll-cushion.v4.${provider}.${channel.toLowerCase()}`;
/** Segments back from the live edge to sit on a non-Twitch stream, and the bounds
 *  that keeps it inside. hls.js's own default is 3, which is the right shape: it
 *  scales with the platform's segment length instead of assuming Twitch's. */
// 1.6 segments, not 3. Three segments is hls.js's conservative default and on
// YouTube's ~5s segments it parked the viewer 15s behind live. A whole segment
// must exist before it can be fetched at all, so ~1.5 is the practical floor for
// plain (non-LL) HLS; sitting just past it is the tightest ride that still has
// something in reserve. The stall ramp below is what makes this safe: a channel
// that cannot sustain it climbs until it is smooth, and remembers.
const PROVIDER_LIVE_SYNC_SEGMENTS = 1.6;
const PROVIDER_LIVE_SYNC_MIN = 4;
const PROVIDER_LIVE_SYNC_MAX = 30;
/** How far above its computed base a provider cushion may ramp on stalls. */
const PROVIDER_LIVE_SYNC_HEADROOM = 6;
/** Used until the manifest reveals the real segment length. Sized for YouTube's
 *  ~5s segments, which is the longest of the platforms in play. */
const PROVIDER_LIVE_SYNC_FALLBACK = 8;

/** The live-edge cushion for a non-Twitch stream, from its real segment length. */
function providerLiveSync(targetDuration: number | undefined): number {
  if (!targetDuration || !Number.isFinite(targetDuration) || targetDuration <= 0) {
    return PROVIDER_LIVE_SYNC_FALLBACK;
  }
  const want = targetDuration * PROVIDER_LIVE_SYNC_SEGMENTS;
  return Math.min(PROVIDER_LIVE_SYNC_MAX, Math.max(PROVIDER_LIVE_SYNC_MIN, want));
}

function learnedLLCushion(
  channel: string | undefined,
  base: number,
  provider = 'twitch',
  headroom = 2,
): number {
  if (!channel) return base;
  try {
    const raw = localStorage.getItem(llCushionKey(channel, provider));
    if (!raw) return base;
    const { c, t } = JSON.parse(raw) as { c: number; t: number };
    if (!Number.isFinite(c) || !Number.isFinite(t)) return base;
    // Decay back toward the base half a second per day, so a channel that was
    // once unstable is retried rather than permanently penalised.
    const days = Math.max(0, (Date.now() - t) / 86_400_000);
    return Math.max(base, Math.min(base + headroom, c - 0.5 * days));
  } catch {
    return base;
  }
}
function rememberLLCushion(channel: string | undefined, cushion: number, provider = 'twitch') {
  if (!channel) return;
  try {
    localStorage.setItem(
      llCushionKey(channel, provider),
      JSON.stringify({ c: cushion, t: Date.now() }),
    );
  } catch {
    /* storage full/blocked: the session keeps its in-memory value */
  }
}

const VideoPlayer = () => {
  const videoRef = useRef<HTMLVideoElement>(null);
  const playerRef = useRef<Plyr | null>(null);
  const hlsRef = useRef<Hls | null>(null);
  // Monotonic token for createPlayer invocations; see the comment at the top
  // of createPlayer. Bumped by every invocation and by effect teardown.
  const createSeqRef = useRef(0);
  // Stops the continuous live-latency governor for the current hls instance. Held
  // in a ref so it survives player recreation and is torn down with the instance.
  const latencyGovernorStopRef = useRef<(() => void) | null>(null);
  const behindLiveWatchdogRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const progressUpdateIntervalRef = useRef<number | null>(null);
  // Actions are stable for the store's lifetime, so take them without
  // subscribing; state goes through a shallow-compared selector. This component
  // is mounted for the whole session, and a bare `useAppStore()` re-rendered it
  // on every unrelated store tick (toasts, mod logs, drops polling).
  const { getAvailableQualities, changeStreamQuality, handleStreamOffline, reloadStreamAndChat, restartStream, exitStream, toggleHome, setHomeActiveTab, setHomeSelectedCategory, createClip, openStreamerMedia } = useAppStore.getState();
  const { streamUrl, settings, activeQuality, adSource, isAutoSwitching, currentStream, isRestartingStream, isHomeActive, streamOriginCategory, isAuthenticated, currentMediaType, isCreatingClip, originalMediaUrl, vodPlayback, liveRewind, liveRewindAvailable, liveRewindAnchor } = useAppStore(
    useShallow((s) => ({
      streamUrl: s.streamUrl,
      settings: s.settings,
      activeQuality: s.activeQuality,
      adSource: s.adSource,
      isAutoSwitching: s.isAutoSwitching,
      currentStream: s.currentStream,
      isRestartingStream: s.isRestartingStream,
      isHomeActive: s.isHomeActive,
      streamOriginCategory: s.streamOriginCategory,
      isAuthenticated: s.isAuthenticated,
      currentMediaType: s.currentMediaType,
      isCreatingClip: s.isCreatingClip,
      originalMediaUrl: s.originalMediaUrl,
      vodPlayback: s.vodPlayback,
      liveRewind: s.liveRewind,
      liveRewindAvailable: s.liveRewindAvailable,
      liveRewindAnchor: s.liveRewindAnchor,
    })),
  );
  // Which platform is playing. Twitch runs the full path below; the ad-source
  // badge, clipping, drops, entitlement and the low-latency origin are all
  // Twitch-contractual and stay gated on this.
  const isTwitchStream = streamProvider(currentStream) === 'twitch';
  // Platform vocabulary. On YouTube the free relationship is "Subscribe" and the
  // paid one is "Join", the inverse of Twitch's words, so a button labelled
  // "Subscribe" there would point a viewer at the paid page.
  const terms = platformTerms(streamProvider(currentStream));
  // Clippable: a live broadcast, or any VOD that's loaded — including the latest
  // VOD auto-loaded into the offline-chat space (still currentMediaType
  // 'offline_chat', but a real VOD is playing, exposed via originalMediaUrl).
  // Clip creation is a Helix call, so it is Twitch-only.
  const canClip =
    isTwitchStream &&
    (currentMediaType === 'live' || (!!originalMediaUrl && /\/videos\/\d+/.test(originalMediaUrl)));
  // A clip is playing in the centered overlay modal. The live stream keeps
  // playing underneath, so mute it while the modal is open to avoid two audio
  // tracks at once; restore the prior mute state on close.
  const clipModalOpen = useAppStore((s) => s.clipModal !== null);
  // Stabilize handleStreamOffline in a ref so createPlayer's identity stays stable.
  // Without this, every Zustand set() call recreates handleStreamOffline, which changes
  // createPlayer's reference, which re-fires the player creation effect — causing double
  // player creation and breaking WebView2's hardware decoder for clip playback.
  const handleStreamOfflineRef = useRef(handleStreamOffline);
  handleStreamOfflineRef.current = handleStreamOffline;
  const playerSettings = settings.video_player;
  // Store settings in a ref so createPlayer doesn't need to depend on them
  // This prevents player recreation when volume/muted settings change
  const playerSettingsRef = useRef(playerSettings);
  playerSettingsRef.current = playerSettings;
  // Volume readout for mouse-driven changes, and the wheel accumulator that
  // turns a trackpad's stream of tiny deltas into whole volume steps.
  const { osd, showOsd } = useVolumeOsd();
  const wheelAccumRef = useRef(createWheelAccumulator());
  // Store isAutoSwitching in a ref so createPlayer can access the latest value
  // without triggering function recreation (which causes black screen/audio glitch)
  const isAutoSwitchingRef = useRef(isAutoSwitching);
  isAutoSwitchingRef.current = isAutoSwitching;
  const isLiveRef = useRef<boolean>(true);
  const userInitiatedPauseRef = useRef<boolean>(false);
  const [availableQualities, setAvailableQualities] = useState<string[]>([]);
  // Tracked as STATE (not just the playerRef) so the quality-menu effect
  // re-runs the moment the player is created. The quality list now resolves
  // instantly, so it can arrive before Plyr exists; gating the menu on a ref
  // alone meant the effect fired once (no player) and never again.
  const [playerReady, setPlayerReady] = useState(false);
  // Overlay visibility state (works in both normal and fullscreen modes)
  const [showOverlay, setShowOverlay] = useState(false);
  // Live telemetry panel ("behind live" + FPS). Toggled from the Plyr settings
  // menu's "Stats" item; the ref lets the menu injection read the latest value.
  const [showStats, setShowStats] = useState(false);
  const showStatsRef = useRef(showStats);
  showStatsRef.current = showStats;
  const overlayTimerRef = useRef<NodeJS.Timeout | null>(null);
  const OVERLAY_HIDE_DELAY = 2600; // Match Plyr's native control hide timing (2.6s)

  // Mute the background stream while a clip modal is open (the clip carries its
  // own audio); restore the prior mute state when it closes.
  const preClipMuteRef = useRef<boolean | null>(null);
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    if (clipModalOpen) {
      if (preClipMuteRef.current === null) preClipMuteRef.current = video.muted;
      video.muted = true;
    } else if (preClipMuteRef.current !== null) {
      video.muted = preClipMuteRef.current;
      preClipMuteRef.current = null;
    }
  }, [clipModalOpen]);

  // Route the live audio through the optional compressor + makeup-gain graph.
  // Re-applied whenever the audio-boost settings change and after each stream
  // swap (the player is rebuilt then, but the <video> element itself persists,
  // so its one-time audio tap stays valid and we just reconfirm the routing).
  // While the feature has never been turned on, this is a no-op and playback is
  // left completely untouched. Scoped to the main player; MultiNook tiles keep
  // their own per-tile audio.
  const audioBoostSettings = playerSettings?.audio_boost;
  useEffect(() => {
    applyAudioBoost(videoRef.current, resolveAudioBoost(audioBoostSettings));
  }, [audioBoostSettings, streamUrl, playerReady]);

  // Expose the player element so the "/song" chat command (which runs outside
  // this component) can capture from the stream that's actually playing.
  useEffect(() => {
    setActiveVideo(videoRef.current);
    return () => setActiveVideo(null);
  }, [streamUrl, playerReady]);

  // Resolved current settings (defaults filled in) drive both the control-bar
  // toggle's appearance and the in-player popover below. The ref lets the
  // injection effect paint the button's initial state without re-running on
  // every toggle.
  const resolvedBoost = resolveAudioBoost(audioBoostSettings);
  const audioBoostEnabled = resolvedBoost.enabled;
  const audioBoostEnabledRef = useRef(audioBoostEnabled);
  audioBoostEnabledRef.current = audioBoostEnabled;

  // In-player popover for editing boost/compressor on the fly without leaving the
  // stream. Opened by right-clicking the control-bar toggle (left-click still
  // does the quick on/off).
  const [audioPanelOpen, setAudioPanelOpen] = useState(false);
  const audioPanelRef = useRef<HTMLDivElement>(null);

  // Write a patch to the persisted audio_boost settings. Reads fresh state so a
  // rapid edit never clobbers a concurrent change; the apply effect and the
  // popover both react to the result.
  const applyBoostPatch = (patch: Partial<AudioBoostSettings>) => {
    const { settings: s, updateSettings } = useAppStore.getState();
    const current = s.video_player;
    const resolved = resolveAudioBoost(current?.audio_boost);
    updateSettings({
      ...s,
      video_player: { ...current, audio_boost: { ...resolved, ...patch } },
    });
  };

  // Inject the Audio Boost toggle into Plyr's control bar, right after the volume
  // group. Plyr's `controls` option only accepts its built-in items, so (like the
  // quality menu and the Stats item) the button is added to the DOM directly.
  // Re-runs whenever Plyr is (re)created (playerReady cycles false->true on every
  // stream swap); the control bar can appear after this fires, so it retries
  // until the DOM exists and guards against double-insertion.
  useEffect(() => {
    if (!playerReady) return;
    const container = containerRef.current;
    if (!container) return;
    let attempts = 0;
    let cancelled = false;
    const inject = () => {
      if (cancelled) return;
      const controls = container.querySelector('.plyr__controls');
      if (!controls) {
        if (attempts++ < 25) setTimeout(inject, 200);
        return;
      }
      if (controls.querySelector('[data-streamnook-audioboost]')) return; // already present

      const btn = document.createElement('button');
      btn.className = 'plyr__controls__item plyr__control';
      btn.type = 'button';
      btn.setAttribute('data-streamnook-audioboost', '');
      btn.innerHTML = `
        <svg class="plyr__icon" aria-hidden="true" focusable="false" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M2 13a2 2 0 0 0 2-2V7a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0V4a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0v-4a2 2 0 0 1 2-2"></path>
        </svg>
        <span class="plyr__tooltip" role="tooltip">Audio Boost</span>
      `;
      // Click opens the in-player popover (which holds the on/off toggle plus
      // all the faders), so everything is adjustable on the fly.
      btn.addEventListener('click', () => {
        setAudioPanelOpen((o) => !o);
      });

      // Sit immediately after the volume group (mute + slider) when present.
      const volume = controls.querySelector('.plyr__volume');
      if (volume && volume.parentElement === controls) {
        volume.insertAdjacentElement('afterend', btn);
      } else {
        const menu = controls.querySelector('.plyr__menu');
        if (menu) controls.insertBefore(btn, menu);
        else controls.appendChild(btn);
      }
      paintAudioBoostButton(btn, audioBoostEnabledRef.current);
    };
    inject();
    return () => {
      cancelled = true;
    };
  }, [playerReady]);

  // Keep the injected toggle's color, tooltip and pressed state in sync whenever
  // Audio Boost is flipped (from this button, the settings panel, or anywhere).
  useEffect(() => {
    const btn = containerRef.current?.querySelector('[data-streamnook-audioboost]');
    paintAudioBoostButton(btn ?? null, audioBoostEnabled);
  }, [audioBoostEnabled, playerReady]);

  // Close the popover on Escape or an outside click (but not on the toggle
  // itself, so right-click can open/close it cleanly). Capture phase so it runs
  // before the player's own pointer handlers.
  useEffect(() => {
    if (!audioPanelOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setAudioPanelOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      const panel = audioPanelRef.current;
      const btn = containerRef.current?.querySelector('[data-streamnook-audioboost]');
      const target = e.target as Node;
      if (panel && !panel.contains(target) && !(btn && btn.contains(target))) {
        setAudioPanelOpen(false);
      }
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onDown, true);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown, true);
    };
  }, [audioPanelOpen]);

  // Close the popover when the stream changes (the control bar is rebuilt then).
  useEffect(() => {
    setAudioPanelOpen(false);
  }, [streamUrl]);

  // Transient top-left "stream note" (ad source + any quality fallback). Shows
  // briefly when a new live stream resolves, then fades. Replaces the per-stream
  // toasts, which fired on nearly every stream and were overbearing.
  const [showStreamNote, setShowStreamNote] = useState(false);
  useEffect(() => {
    if (!(streamUrl && currentMediaType === 'live' && adSource)) {
      setShowStreamNote(false);
      return;
    }
    setShowStreamNote(true);
    const t = setTimeout(() => setShowStreamNote(false), 6000);
    return () => clearTimeout(t);
  }, [streamUrl, currentMediaType, adSource]);

  // Memory Leak Prevention Refs for closures / timeouts
  const volumeDebounceRef = useRef<NodeJS.Timeout | null>(null);
  const bufferGateTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const onPlayingRef = useRef<(() => void) | null>(null);
  const onLoadedMetadataRef = useRef<(() => void) | null>(null);
  // Live-only: a backward scrub past what the live window can serve hands
  // off into the broadcast recording (see the seeking handler in createPlayer).
  const onSeekingRef = useRef<(() => void) | null>(null);
  const onTimeUpdateRef = useRef<(() => void) | null>(null);
  const lastPlayheadRef = useRef<number>(0);

  // Follow + subscribe state and actions for the current channel. Shared with
  // the focused MultiNook tile via useChannelSocial so both overlays behave
  // identically (follow/unfollow, subscribe window, resub/gift detection).
  const {
    isFollowing,
    followLoading,
    checkingFollowStatus,
    heartDropAnimation,
    handleFollowClick,
    isSubscribed,
    hasSubHistory,
    cumulativeMonths,
    subscriberBadgeUrl,
    handleSubscribeClick,
    offersMembership,
    membershipTier,
    membershipDuration,
  } = useChannelSocial({
    userId: currentStream?.user_id,
    userLogin: currentStream?.user_login,
    userName: currentStream?.user_name,
  });

  // Restart stream state
  const [isRestarting, setIsRestarting] = useState(false);

  // Song identification: capture a few seconds of player audio and name the
  // track. The button surfaces the result as a toast (it isn't tied to chat);
  // the /song chat command posts the same result into chat instead.
  const [isIdentifyingSong, setIsIdentifyingSong] = useState(false);
  const handleIdentifySong = useCallback(async () => {
    const addToast = useAppStore.getState().addToast;
    setIsIdentifyingSong(true);
    try {
      const result = await recognizeNowPlaying(videoRef.current);
      // Also drop the clickable card into chat (where the links live); the toast
      // below is just the quick on-player confirmation.
      announceSong(result);
      if (result.status === 'match') {
        const s = result.song;
        const provider = s.providers[0];
        const link = provider?.url || s.song_link || s.shazam_url || undefined;
        addToast(
          `♪ ${s.title} by ${s.artist}${s.album ? ` (${s.album})` : ''}`,
          'success',
          link
            ? {
                label: provider ? `Open in ${provider.name}` : 'Open',
                onClick: () => {
                  void openExternalUrl(link);
                },
              }
            : undefined,
        );
      } else if (result.status === 'no-match') {
        addToast("Couldn't identify the song. Try again when the music is clearer.", 'info');
      } else {
        addToast(result.message, 'error');
      }
    } finally {
      setIsIdentifyingSong(false);
    }
  }, []);

  // Which overlay action buttons the user keeps (undefined = all). Each button
  // still respects its own context gate below (clippable, live-only, etc.).
  const overlayButtonOn = (id: string) =>
    playerOverlayButtonOn(settings.player_overlay_buttons, id);

  // Share the currently-playing channel. Copies the streamnook.app/w/<channel>
  // link, then flips the button to a check for a beat as confirmation.
  const [shareCopied, setShareCopied] = useState(false);
  const handleSharePlayer = async () => {
    if (!currentStream?.user_login) return;
    try {
      await navigator.clipboard.writeText(buildShareUrl(currentStream.user_login));
      setShareCopied(true);
      window.setTimeout(() => setShareCopied(false), 1400);
    } catch (err) {
      Logger.error('[VideoPlayer] Failed to copy share link:', err);
    }
  };

  // Track consecutive fatal errors to determine when stream is truly offline
  const fatalErrorCountRef = useRef<number>(0);
  const manifestErrorCountRef = useRef<number>(0);
  const nonFatalErrorCountRef = useRef<number>(0); // Track non-fatal errors like bufferStalled, fragParsing
  const lastErrorTimeRef = useRef<number>(0);
  const lastSuccessfulPlayRef = useRef<number>(0);
  // Pending "playback paused" report. A `pause` event is debounced before it
  // gates the minute-watched heartbeat off, because ad-bypass region pivots (and
  // quality changes) reload the source via video.pause()/load(), which fires a
  // transient `pause` immediately followed by `playing`. Reporting paused on that
  // blip would stall channel-points/drops earning every ad break. Only a pause
  // that outlives the grace window (a real user pause / dead playback) reports.
  const playingPausedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastFragLoadedTimeRef = useRef<number>(0); // Track when we last received a fragment
  const maxFatalErrorsBeforeOffline = 5; // Increased from 3 - be more tolerant
  const maxManifestErrorsBeforeOffline = 4; // Increased from 2 - be more tolerant
  const maxNonFatalErrorsBeforeOffline = 8; // Non-fatal errors threshold (bufferStalled, fragParsing)
  const errorResetTimeMs = 30000; // Reset error count after 30 seconds of stability
  const minPlayTimeBeforeOfflineMs = 5000; // Must have played at least 5 seconds before considering offline
  const noFragmentTimeoutMs = 15000; // If no fragments received for 15 seconds with errors, stream is likely offline

  // Drop any pending paused-report on unmount so it can't fire after the player
  // is gone and wrongly gate a newly opened stream's earning off.
  useEffect(() => {
    return () => {
      if (playingPausedTimerRef.current) {
        clearTimeout(playingPausedTimerRef.current);
        playingPausedTimerRef.current = null;
      }
    };
  }, []);

  // Fetch available qualities from the resolver (live streams only)
  // Clips and VODs don't need quality switching — they serve a single MP4.
  useEffect(() => {
    if (streamUrl && currentMediaType === 'live') {
      getAvailableQualities().then(qualities => {
        if (qualities.length > 0) {
          setAvailableQualities(qualities);
          Logger.debug('[Quality] Fetched from resolver:', qualities);
        }
      });
    } else {
      // Clear stale qualities when switching to clip/VOD
      setAvailableQualities([]);
    }
  }, [streamUrl, getAvailableQualities, currentMediaType]);

  // Update quality menu when qualities are available
  const updateQualityMenu = useCallback(() => {
    const container = containerRef.current;
    if (!container || availableQualities.length === 0) return;

    // Reflect the quality actually playing (activeQuality) in the menu state,
    // not the saved preference (settings.quality). They diverge whenever the
    // resolver fell back to the closest available quality because the
    // saved preference wasn't offered for this stream.
    const displayedQuality = activeQuality ?? settings.quality;

    Logger.debug('[Quality] Setting up menu with available qualities:', availableQualities);
    Logger.debug('[Quality] Active quality:', activeQuality, 'saved preference:', settings.quality);

    let attempts = 0;
    const buildMenu = () => {
      const settingsMenu = container.querySelector('.plyr__menu');
      if (!settingsMenu) {
        // Plyr is created on hls.js MANIFEST_PARSED, which can land AFTER the
        // quality list resolves (resolution is instant now). Retry until the
        // menu DOM exists instead of bailing on the first miss.
        if (attempts++ < 25) {
          setTimeout(buildMenu, 200);
        } else {
          Logger.warn('[Quality] Could not find Plyr settings menu');
        }
        return;
      }

      // Remove existing quality menu if any
      const existingQualityMenu = settingsMenu.querySelector('[data-quality-menu]');
      if (existingQualityMenu) {
        existingQualityMenu.remove();
      }

      const existingQualityButton = settingsMenu.querySelector('[data-plyr="quality"]');
      if (existingQualityButton) {
        existingQualityButton.remove();
      }

      // Determine the display value - use the actual quality being streamed
      const currentQualityDisplay = displayedQuality.charAt(0).toUpperCase() + displayedQuality.slice(1);

      // Create quality menu item in main settings
      const settingsHome = settingsMenu.querySelector('[role="menu"]');
      if (settingsHome) {
        const qualityMenuItem = document.createElement('button');
        qualityMenuItem.className = 'plyr__control';
        qualityMenuItem.setAttribute('data-plyr', 'quality');
        qualityMenuItem.setAttribute('type', 'button');
        qualityMenuItem.setAttribute('role', 'menuitem');
        qualityMenuItem.innerHTML = `
          <span>Quality<span class="plyr__menu__value">${currentQualityDisplay}</span></span>
        `;

        qualityMenuItem.addEventListener('click', () => {
          const qualitySubmenu = settingsMenu.querySelector('[data-quality-menu]');
          if (qualitySubmenu) {
            settingsHome.setAttribute('hidden', '');
            qualitySubmenu.removeAttribute('hidden');
          }
        });

        // Insert before speed option
        const speedOption = settingsHome.querySelector('[data-plyr="speed"]');
        if (speedOption) {
          settingsHome.insertBefore(qualityMenuItem, speedOption);
        } else {
          settingsHome.appendChild(qualityMenuItem);
        }

        // Create quality submenu (as sibling to settingsHome, not a new menu container)
        const qualitySubmenu = document.createElement('div');
        qualitySubmenu.setAttribute('role', 'menu');
        qualitySubmenu.setAttribute('data-quality-menu', '');
        qualitySubmenu.setAttribute('hidden', '');
        qualitySubmenu.innerHTML = `
          <button class="plyr__control plyr__control--back" type="button" data-plyr="back">
            <span>Quality</span>
          </button>
          ${availableQualities.map(quality => `
            <button 
              class="plyr__control" 
              type="button" 
              data-quality="${quality}"
              role="menuitemradio"
              aria-checked="${quality.toLowerCase() === displayedQuality.toLowerCase() ? 'true' : 'false'}"
            >
              <span>${quality.charAt(0).toUpperCase() + quality.slice(1)}</span>
            </button>
          `).join('')}
        `;

        // Add as sibling to settings home inside the menu container
        const menuContainer = settingsMenu.querySelector('.plyr__menu__container');
        if (menuContainer) {
          menuContainer.appendChild(qualitySubmenu);
        }

        // Handle back button
        const backBtn = qualitySubmenu.querySelector('[data-plyr="back"]');
        backBtn?.addEventListener('click', () => {
          qualitySubmenu.setAttribute('hidden', '');
          settingsHome.removeAttribute('hidden');
        });

        // Handle quality selection
        qualitySubmenu.querySelectorAll('[data-quality]').forEach(btn => {
          if (btn.getAttribute('data-plyr') === 'back') return;

          btn.addEventListener('click', async () => {
            const selectedQuality = btn.getAttribute('data-quality');
            if (!selectedQuality) return;

            Logger.debug(`[Quality] User selected: ${selectedQuality}`);

            // Update UI - mark selected
            qualitySubmenu.querySelectorAll('[data-quality]').forEach(b => {
              if (b.getAttribute('data-plyr') === 'back') return;
              b.setAttribute('aria-checked', 'false');
            });
            btn.setAttribute('aria-checked', 'true');

            // Update value in main menu (with capitalization)
            const qualityValueSpan = settingsHome.querySelector('[data-plyr="quality"] .plyr__menu__value');
            if (qualityValueSpan) {
              qualityValueSpan.textContent = selectedQuality.charAt(0).toUpperCase() + selectedQuality.slice(1);
            }

            // Close menu
            qualitySubmenu.setAttribute('hidden', '');
            settingsHome.removeAttribute('hidden');

            // Re-resolve the stream at the chosen quality.
            await changeStreamQuality(selectedQuality);
          });
        });
      }
    };
    buildMenu();
  }, [availableQualities, settings.quality, activeQuality, changeStreamQuality]);

  // Update time display for live streams: "LIVE" at the edge, the broadcast's
  // elapsed time at the playhead when behind it.
  const liveTimeNodeRef = useRef<Element | null>(null);
  const liveStartedAtRef = useRef<{ raw: string; ms: number } | null>(null);
  // Smoothed distance from the live edge. The raw `buffered.end - currentTime`
  // is a sawtooth that swings by a whole segment, which made this label flip
  // between LIVE and a timestamp, and the timestamp itself jump backwards,
  // several times a minute on a perfectly healthy stream. See utils/liveEdge.
  const liveEdgeRef = useRef(createLiveEdgeTracker());
  const liveTextComputedAtRef = useRef(0);
  const liveTextRef = useRef('LIVE');
  const liveAtLiveRef = useRef(true);
  const updateLiveTimeDisplay = useCallback(() => {
    const video = videoRef.current;
    const container = containerRef.current;
    if (!video || !container || !isLiveRef.current) return;

    // Backgrounded window: keep the loop alive but skip the DOM work.
    if (isWindowHidden()) {
      progressUpdateIntervalRef.current = requestAnimationFrame(updateLiveTimeDisplay);
      return;
    }

    // This must run EVERY frame, not throttled: Plyr writes its own playback
    // clock into the same node on every timeupdate (~4Hz), so any repair gap
    // longer than a frame leaves Plyr's counter visibly alternating with ours.
    // The per-frame cost is a few object reads and one textContent read (no
    // layout); the expensive parts of the old loop (querySelector per frame,
    // unconditional writes) stay gone via the node cache and the mismatch gate.
    const { currentMediaType, currentStream } = useAppStore.getState();
    if (currentMediaType !== 'live') {
      isLiveRef.current = false;
      return;
    }

    // The control-bar node is cached; a player rebuild replaces the control
    // bar, so revalidate via isConnected (cheap) and re-query only then.
    let currentTimeDisplay = liveTimeNodeRef.current;
    if (!currentTimeDisplay || !currentTimeDisplay.isConnected) {
      currentTimeDisplay = container.querySelector('.plyr__time--current');
      liveTimeNodeRef.current = currentTimeDisplay;
    }
    if (currentTimeDisplay) {
      // The VALUE only changes about once a second, but this loop runs every
      // frame to repair Plyr's clobbering (below). Recomputing per frame is
      // what let the raw sawtooth reach the screen. Sample at 4 Hz, cache the
      // result, and let the per-frame half do nothing but repair.
      const now = performance.now();
      if (now - liveTextComputedAtRef.current >= 250) {
        liveTextComputedAtRef.current = now;
        let nextText = 'LIVE';
        const nextAtLive = !liveEdgeRef.current.isBehind(video);
        if (!nextAtLive) {
          const behindSeconds = Math.floor(liveEdgeRef.current.behind(video));
          // Position in the broadcast's elapsed timeline (uptime minus how far
          // behind the edge the playhead sits), like a VOD timestamp for the
          // point being watched. started_at parse is memoized on the string.
          const raw = currentStream?.started_at;
          if (raw) {
            if (liveStartedAtRef.current?.raw !== raw) {
              liveStartedAtRef.current = { raw, ms: Date.parse(raw) };
            }
            const startedMs = liveStartedAtRef.current.ms;
            if (Number.isFinite(startedMs)) {
              const elapsed = Math.max(
                0,
                Math.floor((Date.now() - startedMs) / 1000) - behindSeconds,
              );
              const h = Math.floor(elapsed / 3600);
              const m = Math.floor((elapsed % 3600) / 60);
              const s = elapsed % 60;
              nextText =
                h > 0
                  ? `${h}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`
                  : `${m}:${s.toString().padStart(2, '0')}`;
            }
          }
          if (nextText === 'LIVE') {
            // No usable start time: fall back to the behind-the-edge delta.
            const mins = Math.floor(behindSeconds / 60);
            const secs = behindSeconds % 60;
            nextText = `-${mins}:${secs.toString().padStart(2, '0')}`;
          }
        }
        liveTextRef.current = nextText;
        liveAtLiveRef.current = nextAtLive;
      }
      const nextText = liveTextRef.current;
      const atLive = liveAtLiveRef.current;
      // Compare against what is ACTUALLY in the DOM, never against a cached
      // copy of our own last write, so a Plyr clobber is caught and repaired
      // on the next frame. Reading textContent does not force layout; the
      // mismatch gate keeps steady-state writes at zero.
      if (currentTimeDisplay.textContent !== nextText) {
        currentTimeDisplay.textContent = nextText;
      }
      if (currentTimeDisplay.classList.contains('plyr__time--live') !== atLive) {
        currentTimeDisplay.classList.toggle('plyr__time--live', atLive);
      }
    }

    // Continue the animation loop
    if (isLiveRef.current) {
      progressUpdateIntervalRef.current = requestAnimationFrame(updateLiveTimeDisplay);
    }
  }, []);

  const createPlayer = useCallback(async () => {
    if (!videoRef.current || !streamUrl) return;

    // Invalidation token closing the async-creation race: the awaits below
    // (LL probes) sit between the destroy-existing phase and `new Hls`, so two
    // overlapping invocations (React StrictMode's dev double-mount, or a fast
    // channel swap racing the deferred recreate) both pass the destroy phase
    // and BOTH construct — the one losing the ref assignment became an
    // unkillable zombie player that kept downloading the stream forever.
    // Proven live 2026-06-12: every relay playlist request arrived twice, the
    // duplicates serialized on the socket pool and stacked blocking holds past
    // hls.js's reload budget (the recurring levelLoadTimeOut pairs), and every
    // segment was fetched twice (the phantom bandwidth behind the catch-up
    // spiral). Any superseding invocation or teardown bumps the sequence; a
    // stale invocation aborts at the next checkpoint.
    const seq = ++createSeqRef.current;
    const video = videoRef.current;

    // Clear previously attached video listeners to prevent memory leaks from stacking closures
    if (onPlayingRef.current) video.removeEventListener('playing', onPlayingRef.current);
    if (onLoadedMetadataRef.current) video.removeEventListener('loadedmetadata', onLoadedMetadataRef.current);
    if (onSeekingRef.current) video.removeEventListener('seeking', onSeekingRef.current);
    if (onTimeUpdateRef.current) video.removeEventListener('timeupdate', onTimeUpdateRef.current);
    
    // Clear pending timeouts
    if (bufferGateTimeoutRef.current) {
      clearTimeout(bufferGateTimeoutRef.current);
      bufferGateTimeoutRef.current = null;
    }
    if (volumeDebounceRef.current) {
      clearTimeout(volumeDebounceRef.current);
      volumeDebounceRef.current = null;
    }

    // Read settings from ref to avoid dependency on playerSettings
    // This prevents player recreation when only volume/muted changes
    const currentSettings = playerSettingsRef.current;

    // Destroy existing HLS instance if any
    if (hlsRef.current) {
      try {
        hlsRef.current.destroy();
      } catch (e) {
        Logger.warn('Error destroying existing HLS:', e);
      }
      hlsRef.current = null;
    }

    // ALWAYS destroy existing Plyr instances before recreating!
    // This is explicitly required because Live streams and Clips/VODs have fundamentally
    // different control layouts (progress bars vs none). Reusing instances breaks the UI
    // and can cause phantom black overlays.
    if (playerRef.current) {
      try {
        // CRITICAL: Clear video source BEFORE destroying Plyr.
        // Plyr.destroy() moves the <video> element out of its wrapper back to its
        // original DOM position. If the video is actively decoding (src is set),
        // this DOM re-parenting causes WebView2's hardware video decoder to lose
        // its DirectComposition surface — resulting in audio-but-no-video (black frame).
        video.pause();
        video.removeAttribute('src');
        video.load();
        playerRef.current.destroy();
      } catch (e) {
        Logger.warn('Error destroying existing Plyr:', e);
      }
      playerRef.current = null;
      setPlayerReady(false);
    }

    Logger.debug('Creating HLS.js player for URL:', streamUrl);

    if (streamUrl === 'offline') {
      Logger.debug('[Media] Stream is offline, skipping player creation');
      return;
    }

    // Check if it's an MP4 file
    let urlWithoutQuery = streamUrl.toLowerCase();
    const queryIndex = urlWithoutQuery.indexOf('?');
    if (queryIndex !== -1) {
      urlWithoutQuery = urlWithoutQuery.substring(0, queryIndex);
    }
    const isMp4 = urlWithoutQuery.endsWith('.mp4');

    if (isMp4) {
      Logger.debug('[Media] MP4 stream detected, bypassing Hls.js and Plyr');
      isLiveRef.current = false; // It's a clip!

      // MP4 clips use NATIVE <video> controls — NO Plyr.
      // Plyr wraps the <video> element by moving it inside a new container div. This DOM
      // re-parenting destroys WebView2's DirectComposition surface for the hardware video
      // decoder, resulting in audio-only playback (black/frozen frame). React 18 StrictMode
      // doubles the damage by running mount→cleanup→mount, causing TWO re-parenting cycles.
      // Native <video> avoids all of this — no DOM moves, no surface loss.
      video.controls = true;
      video.src = streamUrl;
      video.volume = currentSettings.volume;
      video.muted = currentSettings.muted;
      video.load();

      // Persist volume/muted changes back to settings (native controls path)
      const onVolumeChange = () => {
        if (volumeDebounceRef.current) clearTimeout(volumeDebounceRef.current);
        volumeDebounceRef.current = setTimeout(() => {
          const { settings, updateSettings } = useAppStore.getState();
          const current = settings.video_player;
          if (current.volume !== video.volume || current.muted !== video.muted) {
            updateSettings({
              ...settings,
              video_player: { ...current, volume: video.volume, muted: video.muted },
            });
          }
        }, 300);
      };
      video.addEventListener('volumechange', onVolumeChange);

      if (currentSettings.autoplay) {
        const playPromise = video.play();
        if (playPromise !== undefined) {
          playPromise.catch(e => {
            Logger.debug('[Media] MP4 autoplay failed, trying muted:', e);
            video.muted = true;
            video.play().catch(e2 => Logger.debug('[Media] Muted MP4 play also failed:', e2));
          });
        }
      }
      return;
    } else if (Hls.isSupported()) {
      Logger.debug('[HLS] HLS.js is supported, creating player...');

      // Is the relay's parts-based LL-HLS origin active for this channel? It is brought
      // up at stream start (before this runs), and `lowLatencyMode` can only be chosen at
      // construction, so resolve it now. The experimental-low-latency setting drives the
      // backend origin kill switch, so this single signal already reflects it: when the
      // setting is off (or the channel isn't low-latency) the origin is inactive and this
      // is false, and we use the stable whole-segment path (cushion + governor).
      // Skipped entirely for non-Twitch streams: the origin keys on Twitch's
      // prefetch tags, so it is never active for them and the probe would only
      // cost a round trip to reach the same answer. Read at call time (not from
      // the render closure) so this can never act on a stale platform.
      let isLowLatencyChannel = false;
      // VOD playback (finished, still recording, or a live rewind) as Rust
      // reported it via start_stream's `vod`. A recording VOD is a growing
      // EVENT playlist that hls.js reads as live, so every live mechanism
      // below is gated off this: otherwise it forced the playhead back to the
      // tail and killed seeking (GitHub #216).
      const vodStart = useAppStore.getState().vodPlayback;
      const isVodPlayback = !!vodStart;
      if (!isVodPlayback && streamProvider(useAppStore.getState().currentStream) === 'twitch') {
        try {
          isLowLatencyChannel = await invoke<boolean>('get_stream_low_latency');
        } catch { /* command unavailable / stream gone */ }
      }
      // Superseded while awaiting the probe: a newer invocation (or teardown)
      // owns the element now. Constructing would create the zombie player.
      if (seq !== createSeqRef.current) {
        Logger.debug('[HLS] Player init superseded mid-probe; aborting this invocation');
        return;
      }
      Logger.debug(`[HLS] LL-HLS origin active=${isLowLatencyChannel}`);
      setIsLowLatencyPath(isLowLatencyChannel);

      // The viewer's preferred behind-live target (displayed seconds), converted to the
      // real cushion/governor value PER PATH so the displayed number tracks the setting
      // either way: the parts path's overlay subtracts the display calibration (so target
      // a calibration higher), while the plain whole-segment path shows hls.latency
      // directly (so target the number as-is). This applies on EVERY channel — a
      // normal-latency broadcast just can't always sustain the tightest values (whole
      // segments arrive with delivery jitter), so the per-channel stall-adaptive cushion
      // settles it where that channel stays smooth.
      const llTargetDisplayed = currentSettings.ll_target_latency ?? LL_TARGET_DEFAULT;
      const llTargetRaw = isLowLatencyChannel
        ? llTargetDisplayed + LL_DISPLAY_CALIBRATION
        : llTargetDisplayed;

      // Create HLS.js instance with optimized settings
      // Read from the store, not the closure: createPlayer is memoized on
      // [streamUrl] alone to keep its identity stable, so a captured render value
      // could go stale. Same pattern as the provider check above.
      const playbackProvider = streamProvider(useAppStore.getState().currentStream);
      const isTwitchPlayback = playbackProvider === 'twitch';
      // Provider cushions are learned per channel exactly like Twitch's. Tracked
      // here so the LEVEL_LOADED correction and the stall ramp share one base.
      let providerCushionBase = PROVIDER_LIVE_SYNC_FALLBACK;
      const hls = new Hls({
        // A custom logger sink ONLY when diagnostics are enabled; otherwise
        // `false` so hls.js does zero internal logging (no per-line string
        // formatting / mirror work). When on, hls.js explains every internal
        // playhead intervention (live-edge resync, start-position seeks, hole
        // skips) through warn/log lines forwarded to the lldiag capture.
        debug: isLLDiagEnabled() ? {
          trace: () => {},
          debug: () => {},
          // info/log are chatty (per-fragment lines), but the MediaSource-EOS
          // chain documents itself ONLY at this level ("buffer reached EOS",
          // "Queueing EOS", "Calling mediaSource.endOfStream()") — mirror just
          // the lines that can explain a wrongly-ended live stream into the
          // capture (not the console).
          log: (...args: unknown[]) => {
            const msg = args.map(String).join(' ');
            if (/EOS|endOfStream|Media source|ENDLIST|live|detach/i.test(msg)) {
              llDiagNote(msg);
            }
          },
          info: (...args: unknown[]) => {
            const msg = args.map(String).join(' ');
            if (/EOS|endOfStream|Media source|ENDLIST|live|detach/i.test(msg)) {
              llDiagNote(msg);
            }
          },
          warn: (...args: unknown[]) => {
            const msg = args.map(String).join(' ');
            // Benign chatter filtered from the console (kept in captures):
            // - segment boundaries overlap ~1ms and hls.js trims each one;
            // - "Need buffer at X but next unloaded part starts at Y" is its
            //   live-edge wait state — the "next unloaded" bookmark sits at the
            //   in-progress segment's start until that segment completes, so
            //   the message cycles ~2-4s behind the buffer forever by design
            //   (verified non-accumulating across a 70-minute capture).
            if (
              !msg.includes('overlapping between fragments detected') &&
              !msg.includes('but next unloaded part starts at')
            ) {
              Logger.debug('[hls.js]', msg);
            }
            llDiagNote(msg);
          },
          error: (...args: unknown[]) => {
            const msg = args.map(String).join(' ');
            Logger.debug('[hls.js:error]', msg);
            llDiagNote(msg);
          },
        } : false,
        enableWorker: true,
        lowLatencyMode: isLowLatencyChannel, // Per-channel: true only when the relay LL-HLS origin is serving parts. Universal true would activate hls.js's playback-rate controller on normal channels and fight liveLatencyGovernor.
        startFragPrefetch: false, // Disabled: prefetching double-buffers massive TS chunks in V8 heap
        backBufferLength: 30, // Keep 30 seconds of back buffer
        maxBufferLength: currentSettings.max_buffer_length || 30, // Buffer ahead
        maxMaxBufferLength: currentSettings.max_buffer_length || 120, // Max buffer
        maxBufferSize: 60 * 1000 * 1000, // 60 MB
        maxBufferHole: 0.5, // Restored — the 2.0 was masking the premature-play root cause
        highBufferWatchdogPeriod: 2, // Restored — check buffer health every 2s
        nudgeOffset: 0.2, // Restored closer to default — buffer gate fixes the real issue
        nudgeMaxRetry: 3, // Restored to default
        maxFragLookUpTolerance: 0.5, // More tolerant fragment lookup
        // The viewer's Live Edge Gap drives the cushion on EVERY path (the slider isn't
        // gated under the Low Latency toggle). The per-channel learned cushion only
        // raises it above the chosen gap where that channel stalled before (normal-channel
        // delivery jitter), so a capable channel/system holds the gap and a jittery one
        // settles where it's smooth. The Low Latency engine (when on) is what lets the
        // lowest gaps stay smooth on supported channels; without it, low gaps self-limit.
        // Twitch's cushion is a per-channel LEARNED value driven by the viewer's Live
        // Edge Gap. Neither half of that transfers to another platform: the learning
        // is keyed by channel and a YouTube stream is addressed by a VIDEO id that
        // changes every broadcast, and the gap itself is tuned against Twitch's ~2s
        // segments. Applied to YouTube's ~5s segments it asks the player to sit less
        // than one segment from the edge, which is why the playhead was observed
        // riding the buffered end with nothing in reserve.
        //
        // Non-Twitch streams get a segment-COUNT cushion instead, refined from the
        // real target duration once the manifest lands (see LEVEL_LOADED below).
        // This is also what the latency governor targets: its default `getTarget`
        // reads `hls.config.liveSyncDuration`, so a too-small value here was being
        // actively enforced rather than merely allowed.
        liveSyncDuration: isTwitchPlayback
          ? learnedLLCushion(currentStream?.user_login, llTargetRaw)
          : learnedLLCushion(
              currentStream?.user_login,
              PROVIDER_LIVE_SYNC_FALLBACK,
              playbackProvider,
              PROVIDER_LIVE_SYNC_HEADROOM,
            ),
        // NOT a memory knob (backBufferLength / maxBufferLength own that). Per hls.js's
        // latency-controller.ts this is the distance from the edge at which hls.js SEEKS
        // FORWARD to liveSyncPosition by itself. Deliberately left far out of reach on the
        // LL path: liveSyncPosition is clamped to `edge - partTarget`, which on this relay
        // is content the origin has promoted but not published, and seeking there
        // mid-playback is the documented freeze. Must stay > liveSyncDuration.
        liveMaxLatencyDuration: isVodPlayback ? undefined : 60,
        // 1 = hls.js's latency controller is fully inert on EVERY path (its rate is
        // quantized to 0.05 steps — dist ~32618 — and each abrupt step is audible
        // through the pitch corrector as a pop/warble, obvious on music, and reads
        // as a micro-hitch; 86 steps in one capture). liveLatencyGovernor owns
        // catch-up on both paths instead, with a smooth ramp.
        maxLiveSyncPlaybackRate: 1,
        liveDurationInfinity: !isVodPlayback, // Live is endless; a VOD (even one still recording) has a real, growing duration
        // Where a VOD begins: the stored resume position or a live rewind's
        // mapped broadcast position. -1 (live) lets hls.js pick the edge.
        startPosition: isVodPlayback ? (vodStart?.start_position_secs ?? 0) : -1,
        manifestLoadingTimeOut: 10000, // 10s timeout for manifest
        manifestLoadingMaxRetry: 3, // Retry manifest 3 times
        manifestLoadingRetryDelay: 1000, // Wait 1s between retries
        levelLoadingTimeOut: 10000, // 10s timeout for level playlists
        levelLoadingMaxRetry: 4, // Retry level 4 times
        levelLoadingRetryDelay: 1000, // Wait 1s between retries
        fragLoadingTimeOut: 20000, // 20s timeout for fragments
        fragLoadingMaxRetry: 6, // Retry fragments 6 times
        fragLoadingRetryDelay: 1000, // Wait 1s between retries
        startLevel: currentSettings.start_quality || -1, // Start quality level
        // ABR tuning
        abrEwmaDefaultEstimate: 1_500_000, // 1.5Mbps — balanced start to prevent initial bandwidth spike
        abrEwmaFastLive: 3.0, // Fast ABR for live streams
        abrEwmaSlowLive: 9.0, // Slow ABR for live streams
        abrBandWidthFactor: 0.95, // Slightly conservative with bandwidth
        abrBandWidthUpFactor: 0.7, // Cautious when upgrading quality
      });

      // Which delivery path this instance rides, for the stats overlay's latency
      // metric. 'll' = the parts-based LL-HLS origin (hls.latency is honest there, and
      // the overlay subtracts a fixed calibration so the number is Twitch-comparable);
      // 'plain' = the stable whole-segment path (hls.latency shown directly).
      (hls as unknown as { __snPathHint?: string }).__snPathHint = isLowLatencyChannel
        ? 'll'
        : 'plain';

      hlsRef.current = hls;

      // Record playback diagnostics on the LL-origin path (per-track buffer ends,
      // fragment PTS/DTS, latency, errors) to a file for offline drift analysis.
      // Only on the LL path we're investigating; stopped on teardown.
      if (isLowLatencyChannel) {
        const channelLabel = (currentStream?.user_login || 'stream').toString();
        void startLLDiagnostics(hls, video, channelLabel);
      }

      // Continuous live-latency maintenance. hls.js's own catch-up is gated off by
      // lowLatencyMode:false, so without this the playhead drifts further behind
      // live across the session. The governor only nudges playbackRate (never seeks,
      // which would freeze a live stream) and reads its target from
      // hls.config.liveSyncDuration, so it adapts to the adaptive cushion set below.
      if (latencyGovernorStopRef.current) {
        latencyGovernorStopRef.current();
        latencyGovernorStopRef.current = null;
      }
      // The governor targets the FORWARD BUFFER, so it speeds up only when there is
      // excess downloaded content to consume (drift) and stays quiet on tight
      // low-latency delivery where the buffer is naturally small — it cannot starve.
      // It owns catch-up on BOTH paths (hls.js's own controller is disabled via
      // maxLiveSyncPlaybackRate: 1 — its 0.05-quantized rate steps are audible).
      // LL channels get the gentle Twitch-parity profile: 1.03 ceiling with a
      // gradual ramp, engaging only past the delivery-jitter band.
      // Real segment length, learned from the first media playlist. Zero until
      // then, which keeps the governor on its default band.
      let segmentSeconds = 0;
      latencyGovernorStopRef.current = isVodPlayback
        ? null
        : isLowLatencyChannel
        ? startLatencyGovernor(hls, video, {
            label: 'solo-ll',
            // Drive BEHIND-LIVE (the playhead's distance from the live edge) toward the
            // viewer's chosen target by speeding up — NOT by shrinking the buffer.
            // hls.latency is honest here (the LL origin lists only real parts). The stats
            // overlay subtracts a fixed calibration so the DISPLAYED number matches the
            // viewer's setting. The `floor` below protects the buffer, so catch-up never
            // stalls; if the chosen target is too tight for a system the stall-adaptive
            // bump raises it for that channel. Read live (not the construction-time
            // llTargetRaw) so moving the gap slider mid-stream takes effect at once.
            latencyTarget: () =>
              (playerSettingsRef.current.ll_target_latency ?? LL_TARGET_DEFAULT) +
              LL_DISPLAY_CALIBRATION,
            getLatency: () =>
              typeof hls.latency === 'number' && hls.latency > 0 ? hls.latency : null,
            gain: 0.12,
            // 1.05 is the edge of pitch-corrector transparency; the old 1.08 was
            // audible as crackle whenever a large behind-live excess pinned the
            // rate at the ceiling for minutes.
            ceiling: 1.05,
            // The band must clear LL delivery jitter (parts land in ~0.3-0.5s
            // lurches), or the excess crosses it on every tick and the rate
            // micro-hunts forever (1.001-1.014 churn, measured 56 rate writes
            // in 150s) - continuous work for the pitch corrector. Within
            // +-0.75s of target the rate is exactly 1.0; real drift still
            // engages. The old 0.1 held latency tighter than anyone can see
            // at the cost of never letting the audio path rest.
            band: 0.75,
            // Low-buffer protection: when delivery wobbles and the forward buffer dips
            // under the floor, ease down to 0.97x instead of running it dry (a 3%
            // slowdown is imperceptible; it prevents stalls by milliseconds).
            floor: 0.8,
            // Full ceiling only with floor+1.5s of real consumable buffer; a
            // behind-live signal the buffer cannot back (relay at its own edge)
            // resolves to 1.0 instead of overspeed-drain-stall cycles.
            //
            // Do not lower this without a capture in hand. It was cut to 0.75 on
            // the theory that the ramp was throttling normal operation. The next
            // session logged 291 buffer stalls in 4h10m against 6 in a 4h11m
            // session on the same path the day before, and the forward buffer at
            // those stalls measured 0.11s median. Overspeed can only convert
            // buffer that already exists into reduced latency; on this path there
            // is very little of it to spend.
            engageSpan: 1.5,
            slowRate: 0.97,
            tickMs: 500,
            rampStep: 0.01,
            log: Logger.debug,
          })
        : startLatencyGovernor(hls, video, {
            label: 'solo',
            // The band has to clear one whole segment. Delivery adds a segment
            // at a time, so a band narrower than that is overshot on EVERY
            // arrival: the buffer crosses the threshold, the rate goes up, the
            // buffer drains back under, the rate goes down, and it never
            // settles. Observed on a 2s-segment YouTube stream as a 1.000/1.050
            // flip roughly every two seconds. Segment length is only known once
            // a playlist lands, so this is read per tick rather than fixed.
            band: () =>
              segmentSeconds > 0
                ? Math.max(DEFAULT_LATENCY_BAND, segmentSeconds * 1.5)
                : DEFAULT_LATENCY_BAND,
            // And when it does engage, get there gradually. Without a ramp the
            // rate steps the whole way in one tick, which on a 2s cadence is a
            // 5% jump every couple of seconds — the same audible stepping the
            // low-latency profile above uses `rampStep` to avoid. Left off for
            // Twitch (segmentSeconds stays 0 there), whose tuning is unchanged.
            rampStep: () => (segmentSeconds > 0 ? 0.01 : 0),
            log: Logger.debug,
          });

      // A platform's real segment length is only known once a media playlist lands,
      // so the cushion set at construction is a fallback. Correct it here, and only
      // for non-Twitch streams: Twitch's is the viewer's learned Live Edge Gap and
      // must not be overwritten. The governor re-reads this every tick, so raising
      // it takes effect without restarting anything.
      if (!isTwitchPlayback) {
        hls.once(Hls.Events.LEVEL_LOADED, (_e, data) => {
          const target = data?.details?.targetduration;
          // Also what the governor's band is sized against; see above.
          if (typeof target === 'number' && target > 0) segmentSeconds = target;
          providerCushionBase = providerLiveSync(target);
          // Anything this channel had to back off to previously wins over the
          // fresh base, so a known-wobbly stream doesn't re-discover its stalls
          // from scratch every session.
          const cushion = learnedLLCushion(
            currentStream?.user_login,
            providerCushionBase,
            playbackProvider,
            PROVIDER_LIVE_SYNC_HEADROOM,
          );
          if (cushion !== hls.config.liveSyncDuration) {
            hls.config.liveSyncDuration = cushion;
            Logger.debug(
              `[player] provider live cushion ${cushion}s (base ${providerCushionBase}s, targetDuration=${target ?? 'unknown'})`,
            );
          }
        });
      }

      // Behind-live recovery watchdog (LL path only). A transient upstream stall
      // (Twitch's CDN pausing the in-progress segment for >cushion seconds) drains the
      // buffer, stalls, and leaves the playhead re-anchored at the START of the origin's
      // window — ~12s behind live — where rate catch-up alone takes ages, so it rides
      // there forever (observed live). The governor's gentle 1.03 handles small drift,
      // but not a sudden whole-window jump. When the forward buffer sits well past the
      // cushion for a couple of checks, snap back into the buffer to ~cushion behind the
      // edge (the same in-buffer seek Go Live uses — safe because the target is already
      // downloaded; only a truly empty buffer needs a full restart). Cooldown prevents
      // thrashing if upstream is flaky.
      if (behindLiveWatchdogRef.current) {
        clearInterval(behindLiveWatchdogRef.current);
        behindLiveWatchdogRef.current = null;
      }
      if (isLowLatencyChannel) {
        let over = 0;
        let frozen = 0;
        let lastCt = -1;
        let lastRecover = 0;
        behindLiveWatchdogRef.current = setInterval(() => {
          const v = (hls.media as HTMLVideoElement | null) ?? video;
          if (!v || v.paused || v.seeking) {
            over = 0;
            frozen = 0;
            lastCt = -1;
            return;
          }
          const b = v.buffered;
          const end = b.length > 0 ? b.end(b.length - 1) : 0;
          const cushion =
            typeof hls.config.liveSyncDuration === 'number' && Number.isFinite(hls.config.liveSyncDuration)
              ? hls.config.liveSyncDuration
              : 2;
          const fwd = end - v.currentTime;
          // Has the playhead advanced since the last tick? In a healthy ride it
          // moves ~1 tick-worth per tick (≥1.9s at 2s ticks); a real stall pins it.
          const advanced = lastCt >= 0 && v.currentTime - lastCt > 0.05;
          lastCt = v.currentTime;
          // Beyond cushion + 5s = a real fall-behind (OVERSHOOT: lots of reachable
          // buffer ahead). Normal LL forward buffer is ~cushion (2s); a drought
          // re-anchors the playhead to the start of the origin's ~12s window, so the
          // fall-behind sits far above any legit part-burst spike — the +5 keeps brief
          // spikes from triggering a visible snap. A huge value (DVR scrub-back) is the
          // user's, left alone.
          if (fwd > cushion + 5 && fwd < 120) {
            over += 1;
            frozen = 0;
            if (over >= 2 && Date.now() - lastRecover > 15000) {
              lastRecover = Date.now();
              over = 0;
              if (end === 0) {
                restartStream();
              } else {
                const target = end - cushion;
                if (target > v.currentTime + 0.5) {
                  Logger.debug(`[HLS] behind-live watchdog: ${fwd.toFixed(1)}s behind, snapping to edge-${cushion}s`);
                  v.currentTime = target;
                }
              }
            }
          } else if (!advanced && fwd < 1.5) {
            // UNDERSHOOT: the playhead is frozen with no reachable forward buffer —
            // a famine outlasted the local buffer, the player drained to ~0 and
            // hls.js gave up (its catch-up re-anchored to think it is near live, so
            // wallLat then climbs 1s/s forever; observed summit1g 2026-06-12,
            // wallLat 6s -> 78s over 70s with rate pinned at 0.97 and no rescue).
            // The overshoot branch above can never fire here (fwd is below, not
            // above, threshold). After ~8s of a pinned playhead, recover: if fresh
            // live data has landed as a separate buffered range ahead of the hole,
            // snap into it (already downloaded, safe); otherwise restart so the
            // session re-anchors to the live edge instead of drifting forever.
            over = 0;
            frozen += 1;
            if (frozen >= 4 && Date.now() - lastRecover > 15000) {
              lastRecover = Date.now();
              frozen = 0;
              let snapped = false;
              for (let i = 0; i < b.length; i++) {
                // A range strictly ahead of the stuck playhead with real content.
                if (b.start(i) > v.currentTime + 0.1 && b.end(i) - b.start(i) > 0.5) {
                  const target = Math.max(b.start(i) + 0.1, b.end(i) - cushion);
                  Logger.debug(`[HLS] behind-live watchdog: playhead frozen at ${v.currentTime.toFixed(1)}, snapping into live range -> ${target.toFixed(1)}`);
                  v.currentTime = target;
                  snapped = true;
                  break;
                }
              }
              if (!snapped) {
                Logger.debug('[HLS] behind-live watchdog: playhead frozen with no reachable buffer, restarting to re-anchor to live');
                restartStream();
              }
            }
          } else {
            // No third "drifted far behind while advancing" branch here on purpose.
            // One existed and was removed: it triggered off `hls.latency`, which
            // hls.js computes as `levelDetails.edge + levelDetails.age -
            // currentTime` (latency-controller.ts). The `age` term means a playlist
            // that stops updating inflates the reading by one second per second
            // while playback is completely unaffected, so the branch fired on a
            // measurement artifact and its remedy was a full restartStream().
            // Any future drift handling has to measure the SERVABLE distance (what
            // is downloaded ahead of the playhead), not the advertised edge.
            over = 0;
            frozen = 0;
          }
        }, 2000);
      }

      // HLS.js event handlers
      hls.on(Hls.Events.MANIFEST_PARSED, (_event, data) => {
        Logger.debug('[HLS] Manifest parsed, starting playback');
        Logger.debug('[HLS] Available quality levels:', data.levels.map(l => `${l.height}p @ ${l.bitrate}bps`).join(', '));

        // Diagnostic: log the details HLS.js uses for live sync calculations
        const selectedLevel = data.levels[data.firstLevel || 0];
        Logger.debug(
          `[HLS-DIAG] MANIFEST | levels=${data.levels.length} | ` +
          `firstLevel=${data.firstLevel} | ` +
          `targetDuration=${selectedLevel?.details?.targetduration ?? 'N/A'} | ` +
          `liveSyncPosition=${hls.liveSyncPosition?.toFixed(1) ?? 'N/A'} | ` +
          `config.liveSyncDuration=${hls.config.liveSyncDuration ?? 'unset'} | ` +
          `config.liveSyncDurationCount=${hls.config.liveSyncDurationCount ?? 'unset'}`
        );

        // Reset error counts on successful manifest parse - stream is working
        fatalErrorCountRef.current = 0;
        manifestErrorCountRef.current = 0;

        // Adaptive low-latency cushion. The relay scans every playlist and flags
        // low-latency broadcasts; ask it once here (one cheap IPC, no manifest
        // re-download). On a low-latency stream with Low Latency on, tighten the
        // cushion to ~2s.
        //

        // Initialize Plyr AFTER we have the video loaded
        if (!playerRef.current) {
          const player = new Plyr(video, {
            controls: [
              'play-large',
              'play',
              'progress',
              'current-time',
              ...(isVodPlayback ? ['duration'] : []),
              'mute',
              'volume',
              'settings',
              'pip',
              'fullscreen',
            ],
            settings: ['speed'], // Remove 'quality' - we'll add it manually
            speed: {
              selected: 1,
              options: [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2],
            },
            autoplay: false, // Gate controls playback start — wait for enough buffer
            muted: currentSettings.muted,
            volume: currentSettings.volume,
            invertTime: false,
            // Player hotkeys are owned by StreamNook's unified keybinding
            // engine (src/keybindings), not Plyr. Disabling Plyr's own keyboard
            // handling prevents double-firing and keeps every player key
            // rebindable from Settings > Keybindings.
            keyboard: { focused: false, global: false },
            tooltips: { controls: true, seek: true },
            hideControls: true,
            // Single click toggles play/pause, double click toggles fullscreen.
            // Both are driven by a manual click/dblclick listener (see effect
            // below) so a double-click never also flickers play state — Plyr's
            // own click-to-play is disabled here.
            clickToPlay: false,
            // Force Plyr's CSS-only fullscreen — the Tauri window is borderless
            // (decorations: false), so HTML5 element-fullscreen ends up scoped
            // to the window viewport instead of the screen. We bridge Plyr's
            // enter/exit events to the Tauri window's true OS fullscreen below.
            fullscreen: { enabled: true, fallback: 'force', iosNative: false },
            // Disable Plyr's built-in localStorage - we manage settings via Tauri backend
            storage: { enabled: false },
          });

          playerRef.current = player;
          setTimelineHost(containerRef.current?.querySelector<HTMLElement>('.plyr__progress') ?? null);
          setPlayerReady(true);

          player.on('enterfullscreen', () => syncTauriWindowFullscreen(true));
          player.on('exitfullscreen', () => syncTauriWindowFullscreen(false));

          // Set up live stream overrides
          isLiveRef.current = useAppStore.getState().currentMediaType === 'live' && !isVodPlayback;

          // Override duration for live stream progress bar
          if (isLiveRef.current) {
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
          }

          // Initial volume sync
          playerRef.current.volume = currentSettings.volume;
          playerRef.current.muted = currentSettings.muted;

          // Listen for Plyr events
          player.on('play', () => {
            userInitiatedPauseRef.current = false;
          });

          player.on('pause', () => {
            setTimeout(() => {
              if (video.paused) {
                userInitiatedPauseRef.current = true;
              }
            }, 50);
          });

          // Persist volume/muted changes back to settings when user adjusts via Plyr UI
          player.on('volumechange', () => {
            if (!playerRef.current) return;
            const newVolume = playerRef.current.volume;
            const newMuted = playerRef.current.muted;

            // Debounce to avoid hammering the backend on every slider tick
            if (volumeDebounceRef.current) clearTimeout(volumeDebounceRef.current);
            volumeDebounceRef.current = setTimeout(() => {
              const { settings, updateSettings } = useAppStore.getState();
              const current = settings.video_player;
              if (current.volume !== newVolume || current.muted !== newMuted) {
                updateSettings({
                  ...settings,
                  video_player: { ...current, volume: newVolume, muted: newMuted },
                });
              }
            }, 300);
          });
        }
        if (isLowLatencyChannel) {
          // LL path: hls.js fires part-based buffering, not the whole-segment
          // FRAG_BUFFERED our gate listens for, so the gate + its 5s fallback are
          // skipped below. Don't play at manifest parse though: on a warm origin
          // (stream refresh) the manifest lands before hls.js has level details, so
          // playing here rolls the clock from 0 while the first data appends at the
          // live position, stalling at @0 until hls.js's own start seek wins (and
          // our stall recovery fights it with +0.5s nudges). Wait one LEVEL_LOADED
          // (details known), put the playhead at the live sync point, then play.
          hls.once(Hls.Events.LEVEL_LOADED, (_e, data) => {
            const cushion =
              typeof hls.config.liveSyncDuration === 'number' ? hls.config.liveSyncDuration : 2;
            const edge = data.details.edge;
            const pos =
              hls.liveSyncPosition ?? (Number.isFinite(edge) ? Math.max(0, edge - cushion) : null);
            if (pos != null && Number.isFinite(pos) && pos > 0) {
              video.currentTime = pos;
            }
            Logger.debug(
              `[HLS] Level loaded (LL) — starting playback at ${video.currentTime.toFixed(2)}`,
            );
            video.play().catch(() => {
              video.muted = true;
              video.play().catch(() => Logger.debug('[HLS] LL muted autoplay also failed'));
            });
          });
        } else {
          // Non-LL: don't play here. The FRAG_BUFFERED gate below calls play() once
          // enough buffer exists, then snaps to the cushion. Prevents cold-start stall.
          Logger.debug('[HLS] Manifest parsed — waiting for buffer depth before play...');
        }
      });

      hls.on(Hls.Events.ERROR, (_event, data) => {
        Logger.error('[HLS] Error:', JSON.stringify({ type: data.type, details: data.details, fatal: data.fatal }));
        // hls.js packs the precise reason into data.error.message — for a
        // levelParsingError that is the exact parser failure (e.g. "media sequence
        // mismatch …" or "discontinuity sequence mismatch …" with BOTH playlists
        // embedded, or "Missing Target Duration" / "No Segments found"). The line
        // above drops it, which is why level parse failures have been opaque.
        if (data.error?.message) {
          Logger.error('[HLS] Error detail:', data.error.message);
        }

        // Handle non-fatal errors with improved recovery
        if (!data.fatal) {
          const now = Date.now();

          // Handle buffer stalled errors with active recovery
          if (data.details === 'bufferStalledError') {
            Logger.debug('[HLS] Buffer stalled, attempting recovery...');

            // Stall-adaptive cushion (LL path): each real stall proves this
            // session's delivery is wobblier than the cushion tolerates, so
            // raise the target half a second (cap 3s). Clean channels keep the
            // tight ~2-2.5s ride; unstable ones settle where they stop
            // stuttering instead of stalling at a fixed tight target. The
            // governor and watchdog read liveSyncDuration live, and the buffer
            // that accumulates while delivery recovers simply stops being
            // consumed at the higher target — no seek, no visible action.
            // Session-scoped on purpose: a channel that stalled once will
            // likely wobble again, so the cushion does not decay back.
            // Twitch LL and every provider stream both ride close to the edge, so
            // both back off the same way: climb on a stall, remember it, and let
            // it decay back over days. Only Twitch's NON-LL path is excluded,
            // where the cushion is the viewer's own setting rather than something
            // we discovered.
            if (isLowLatencyChannel || !isTwitchPlayback) {
              const fallbackTarget = isTwitchPlayback ? llTargetRaw : providerCushionBase;
              const cur =
                typeof hls.config.liveSyncDuration === 'number' &&
                Number.isFinite(hls.config.liveSyncDuration)
                  ? hls.config.liveSyncDuration
                  : fallbackTarget;
              // Providers get more headroom than Twitch LL: their segments are
              // whole (no parts), so recovery needs a bigger reserve.
              const cap =
                fallbackTarget + (isTwitchPlayback ? 2 : PROVIDER_LIVE_SYNC_HEADROOM);
              if (cur < cap) {
                hls.config.liveSyncDuration = Math.min(cap, cur + 0.5);
                Logger.debug(
                  `[HLS] stall: cushion ${cur.toFixed(1)}s -> ${hls.config.liveSyncDuration.toFixed(1)}s`,
                );
                // Remember per channel so the next session starts here instead
                // of re-discovering the wobble one stall at a time.
                rememberLLCushion(
                  currentStream?.user_login,
                  hls.config.liveSyncDuration,
                  playbackProvider,
                );
              }
            }

            // Try to recover by seeking slightly forward if video is paused
            if (video.paused && !userInitiatedPauseRef.current) {
              Logger.debug('[HLS] Video is paused due to stall, attempting to resume playback');
              video.play().catch(e => Logger.debug('[HLS] Resume play failed:', e));
            }

            // If we have buffered data ahead, try jumping to it. NON-LL path
            // only: on the LL path a forward seek toward the edge mid-playback
            // is the documented freeze (it flushes MSE coded-frame state and
            // invalidates the blocking reload the origin holds for the next
            // part). An LL stall is handled instead by the cushion bump above
            // plus the behind-live watchdog's in-buffer snap, which only ever
            // seeks to already-downloaded content.
            if (!isLowLatencyChannel) {
              const buffered = video.buffered;
              if (buffered.length > 0) {
                const currentTime = video.currentTime;
                const bufferedEnd = buffered.end(buffered.length - 1);

                // If we're significantly behind the buffered end, seek forward
                if (bufferedEnd - currentTime > 2.0) {
                  const seekTarget = currentTime + 0.5; // Small jump forward
                  Logger.debug(`[HLS] Seeking forward from ${currentTime} to ${seekTarget} to recover from stall`);
                  video.currentTime = seekTarget;
                }
              }
            }

            // Reset error count if enough time has passed since last error
            if (now - lastErrorTimeRef.current > errorResetTimeMs) {
              nonFatalErrorCountRef.current = 0;
            }

            nonFatalErrorCountRef.current++;
            lastErrorTimeRef.current = now;

            Logger.debug(`[HLS] Non-fatal error count: ${nonFatalErrorCountRef.current}/${maxNonFatalErrorsBeforeOffline}`);
            Logger.debug(`[HLS] Time since last fragment: ${now - lastFragLoadedTimeRef.current}ms`);

            // Check if we should trigger offline detection
            // Conditions: 
            // 1. Multiple non-fatal errors accumulated
            // 2. No new fragments received for a while (stream is stalled)
            // 3. Stream has been playing successfully before (not initial load issues)
            const hasPlayedSuccessfully = lastSuccessfulPlayRef.current > 0;
            const timeSinceLastFrag = now - lastFragLoadedTimeRef.current;
            const fragmentsStalled = lastFragLoadedTimeRef.current > 0 && timeSinceLastFrag > noFragmentTimeoutMs;

            const shouldTriggerOffline =
              nonFatalErrorCountRef.current >= maxNonFatalErrorsBeforeOffline &&
              hasPlayedSuccessfully &&
              fragmentsStalled &&
              !isAutoSwitchingRef.current;

            if (shouldTriggerOffline) {
              Logger.debug('[HLS] Multiple non-fatal errors with stalled fragments detected. Stream appears to have ended.');
              Logger.debug(`[HLS] Non-fatal errors: ${nonFatalErrorCountRef.current}, Time since frag: ${timeSinceLastFrag}ms`);

              // Reset error counts
              nonFatalErrorCountRef.current = 0;
              fatalErrorCountRef.current = 0;
              manifestErrorCountRef.current = 0;

              // Trigger auto-switch
              handleStreamOfflineRef.current();
              return;
            }
          }

          // Handle other non-fatal errors
          if (data.details === 'fragParsingError' || data.details === 'bufferNudgeOnStall') {
            // Reset error count if enough time has passed since last error
            if (now - lastErrorTimeRef.current > errorResetTimeMs) {
              nonFatalErrorCountRef.current = 0;
            }

            nonFatalErrorCountRef.current++;
            lastErrorTimeRef.current = now;

            Logger.debug(`[HLS] Non-fatal error (${data.details}) count: ${nonFatalErrorCountRef.current}/${maxNonFatalErrorsBeforeOffline}`);
          }

          // For non-fatal errors, return early - HLS.js will try to recover
          return;
        }

        // Handle fatal errors
        if (data.fatal) {
          const now = Date.now();

          // Reset error count if enough time has passed since last error
          if (now - lastErrorTimeRef.current > errorResetTimeMs) {
            fatalErrorCountRef.current = 0;
          }

          fatalErrorCountRef.current++;
          lastErrorTimeRef.current = now;

          Logger.debug(`[HLS] Fatal error count: ${fatalErrorCountRef.current}/${maxFatalErrorsBeforeOffline}`);

          // Check if we've exceeded max fatal errors - likely stream is offline
          if (fatalErrorCountRef.current >= maxFatalErrorsBeforeOffline && !isAutoSwitchingRef.current) {
            Logger.debug('[HLS] Max fatal errors reached, stream appears to be offline. Triggering auto-switch...');

            // Reset error count
            fatalErrorCountRef.current = 0;

            // Trigger auto-switch
            handleStreamOfflineRef.current();
            return;
          }

          switch (data.type) {
            case Hls.ErrorTypes.NETWORK_ERROR:
              Logger.debug('[HLS] Fatal network error, attempting recovery...');

              // Check if this is a manifest/playlist load error (strong indicator of offline stream)
              if (data.details === 'manifestLoadError' ||
                data.details === 'manifestLoadTimeOut' ||
                data.details === 'manifestParsingError' ||
                data.details === 'levelLoadError' ||
                data.details === 'levelLoadTimeOut') {
                Logger.debug(`[HLS] Manifest/level loading failed: ${data.details}`);

                // For manifest errors, use separate counter to be more careful
                manifestErrorCountRef.current++;
                Logger.debug(`[HLS] Manifest error count: ${manifestErrorCountRef.current}/${maxManifestErrorsBeforeOffline}`);

                // Only trigger offline if we've had multiple manifest errors AND 
                // either never played successfully OR it's been a while since we started
                const timeSinceStart = Date.now() - lastSuccessfulPlayRef.current;
                const hasPlayedSuccessfully = lastSuccessfulPlayRef.current > 0;
                const shouldTriggerOffline = manifestErrorCountRef.current >= maxManifestErrorsBeforeOffline &&
                  (!hasPlayedSuccessfully || timeSinceStart > minPlayTimeBeforeOfflineMs);

                if (shouldTriggerOffline && !isAutoSwitchingRef.current) {
                  Logger.debug('[HLS] Multiple manifest errors, stream likely offline. Triggering auto-switch...');
                  Logger.debug(`[HLS] Has played: ${hasPlayedSuccessfully}, Time since start: ${timeSinceStart}ms`);
                  manifestErrorCountRef.current = 0;
                  fatalErrorCountRef.current = 0;
                  handleStreamOfflineRef.current();
                  return;
                }
              }

              hls.startLoad();
              break;
            case Hls.ErrorTypes.MEDIA_ERROR:
              Logger.debug('[HLS] Fatal media error, attempting recovery...');
              hls.recoverMediaError();
              break;
            default:
              Logger.error('[HLS] Fatal error, cannot recover. Recreating player...');
              setTimeout(() => {
                if (videoRef.current && isLiveRef.current && !isAutoSwitchingRef.current) {
                  createPlayer();
                }
              }, 2000);
              break;
          }
        }
      });

      hls.on(Hls.Events.LEVEL_SWITCHED, (_event, data) => {
        const level = hls.levels[data.level];
        Logger.debug(`[HLS] Level switched to: ${data.level} (${level?.height}p @ ${level?.bitrate}bps)`);

        // Update Plyr's quality display if in auto mode
        if (playerRef.current && hls.currentLevel === -1) {
          // In auto mode, update the display to show current quality
          const qualityBadge = containerRef.current?.querySelector('.plyr__menu__container [data-plyr="quality"][aria-checked="true"]');
          if (qualityBadge && level) {
            Logger.debug(`[HLS] Auto selected: ${level.height}p`);
          }
        }
      });

      hls.on(Hls.Events.FRAG_LOADED, (_event, data) => {
        lastFragLoadedTimeRef.current = Date.now();
        nonFatalErrorCountRef.current = 0;
        if (Math.random() < 0.05) {
          Logger.debug(`[HLS] Fragment loaded: sn=${data.frag.sn} dur=${data.frag.duration.toFixed(1)}s`);
        }
      });

      // ──── Cold-Start Buffer Gate + live-sync snap ────
      // Twitch segments are ~2s. Wait for a little buffer so a cold start doesn't
      // stall, THEN snap the playhead to the live-sync point. The snap is the
      // load-bearing part: the buffer this gate accumulates piles up BEHIND the
      // advancing live edge, so without it the cold-start buffer (not
      // liveSyncDuration) decides how far back we play, pinning latency ~6-8s no
      // matter the configured cushion. Snapping to (freshest buffered edge minus
      // liveSyncDuration), clamped into the buffered range, makes the cushion
      // actually control latency. This mirrors a manual "scrub close to live",
      // which sustains without stalling.
      // Threshold is 3, not 4: two 2s segments measure as ~3.96s, which just
      // misses a 4 threshold and forces a ~2s wait for a freshly-produced third
      // segment (the slow-load regression). 3 clears on the two already-available
      // segments instead.
      let playStarted = false;
      // On the LL path hls.js positions at the live edge itself; we only need a tiny
      // buffer before starting and must NOT snap (hls.js owns the playhead). On the
      // non-LL path keep the 3s gate + snap that makes the cushion control latency.
      const GATE_THRESHOLD = isLowLatencyChannel ? 1 : 3;

      hls.on(Hls.Events.FRAG_BUFFERED, () => {
        // LL path started playback in MANIFEST_PARSED and positions via hls.js; the
        // whole-segment gate doesn't apply (and FRAG_BUFFERED is unreliable per-part).
        if (playStarted || !currentSettings.autoplay || isLowLatencyChannel) return;

        const buffered = video.buffered;
        if (buffered.length === 0) return;

        const bufStart = buffered.start(0);
        const bufEnd = buffered.end(buffered.length - 1);
        const depth = bufEnd - bufStart;
        if (depth < GATE_THRESHOLD) return;

        playStarted = true;

        // Snap to the live-sync point (cushion behind the freshest buffered edge),
        // clamped so we never seek into an unbuffered hole. No-op when the buffer
        // is already within the cushion (e.g. low-latency off), so it never pushes
        // anyone CLOSER than their setting asks for. Skipped on the LL path — hls.js's
        // low-latency controller owns positioning there, and a manual seek would fight it.
        const syncDur = hls.config.liveSyncDuration ?? 4;
        const target = Math.max(bufStart, bufEnd - syncDur);
        if (!isLowLatencyChannel && !isVodPlayback && target > video.currentTime + 0.5) {
          video.currentTime = target;
        }

        Logger.debug(`[HLS] Buffer gate cleared: ${depth.toFixed(1)}s buffered, edge ${bufEnd.toFixed(1)}, snap to ${target.toFixed(1)} — starting playback`);
        video.play().catch(e => {
          Logger.debug('[HLS] Autoplay failed:', e);
          video.muted = true;
          video.play().catch(() => Logger.debug('[HLS] Muted autoplay also failed'));
        });
      });

      // Fallback timeout (non-LL only: the LL path already started in MANIFEST_PARSED).
      if (!isLowLatencyChannel) {
        bufferGateTimeoutRef.current = setTimeout(() => {
          if (!playStarted && currentSettings.autoplay) {
            playStarted = true;
            Logger.debug('[HLS] Buffer gate timeout — starting playback');
            video.play().catch(() => {});
          }
        }, 5000);
      }

      // Load the stream
      hls.loadSource(streamUrl);
      hls.attachMedia(video);

      // Add video event listeners
      const onLoadedMetadata = () => {
        Logger.debug(`[Video] Metadata loaded: ${video.videoWidth}x${video.videoHeight}`);
      };

      const onPlaying = () => {
        Logger.debug(`[Video] Playing: ${video.videoWidth}x${video.videoHeight}, paused: ${video.paused}, readyState: ${video.readyState}`);
        Logger.debug(`[Video] Audio state: muted=${video.muted}, volume=${video.volume}`);

        // Track successful playback - this helps us distinguish between
        // "stream never loaded" vs "stream was playing and then had issues"
        if (lastSuccessfulPlayRef.current === 0) {
          lastSuccessfulPlayRef.current = Date.now();
          Logger.debug('[Video] First successful playback recorded');
        }

        // Buffer gate handles the initial seek-to-live — no jump_to_live logic needed here.

        // Reset error counts on successful playback - stream is working
        fatalErrorCountRef.current = 0;
        manifestErrorCountRef.current = 0;
        nonFatalErrorCountRef.current = 0;
      };

      onLoadedMetadataRef.current = onLoadedMetadata;
      onPlayingRef.current = onPlaying;

      video.addEventListener('loadedmetadata', onLoadedMetadata);
      video.addEventListener('playing', onPlaying);

      // Scrubbing BACK on a live Twitch stream. The live window cannot hold a
      // position: the low-latency origin serves ~12 s and its watchdog snaps
      // a lagging playhead to the edge within seconds, and the whole-segment
      // path is force-seeked by hls.js past 60 s behind. So a deliberate
      // backward scrub past what the buffer already holds becomes a rewind
      // into the broadcast recording (Rust maps "this far behind live" onto
      // the VOD), with "Back to live" to return. Every programmatic seek in
      // this file moves FORWARD (edge snaps, stall nudges, Go Live), so
      // "backward by more than a few seconds" is the user's own intent.
      lastPlayheadRef.current = 0;
      const onTimeUpdate = () => {
        if (!video.seeking) lastPlayheadRef.current = video.currentTime;
      };
      const onSeeking = () => {
        if (isVodPlayback || !isTwitchPlayback) return;
        const store = useAppStore.getState();
        if (store.currentMediaType !== 'live' || store.liveRewind || store.isRestartingStream) return;
        const from = lastPlayheadRef.current;
        const to = video.currentTime;
        if (!(from > 0) || to > from - LIVE_SCRUB_HANDOFF_SECS) return;
        const b = video.buffered;
        const edge = b.length > 0 ? b.end(b.length - 1) : from;
        const held = b.length > 0 ? b.start(0) : from;
        // Inside what is already buffered (and, off the LL path, inside the
        // 60 s hls.js tolerates) the live player can serve the scrub itself.
        if (to >= held && (isLowLatencyChannel ? false : edge - to < LIVE_DVR_MAX_BEHIND_SECS)) return;
        const behind = Math.max(0, edge - to);
        if (store.liveRewindAvailable === false) {
          // Nothing to rewind into: keep the live position and say why, once
          // per scrub, instead of silently snapping back to the edge.
          video.currentTime = from;
          store.addToast(`${store.currentStream?.user_name || 'This channel'} has VODs turned off, so the broadcast can't be rewound`, 'info');
          return;
        }
        Logger.debug(`[player] live scrub back ${behind.toFixed(0)}s past the live window; rewinding into the recording`);
        // Hold the playhead where it was so nothing plays from the dead spot
        // while Rust resolves the recording (~1 s); the relay swap recreates
        // the player at the mapped position.
        video.currentTime = from;
        void store.rewindLive({ behindSecs: behind });
      };
      onTimeUpdateRef.current = onTimeUpdate;
      onSeekingRef.current = onSeeking;
      video.addEventListener('timeupdate', onTimeUpdate);
      video.addEventListener('seeking', onSeeking);

    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
      // Native HLS support (Safari)
      Logger.debug('[HLS] Using native HLS support');
      video.src = streamUrl;

      // Initialize Plyr for Safari
      const player = new Plyr(video, {
        controls: [
          'play-large',
          'play',
          'progress',
          'current-time',
          'mute',
          'volume',
          'settings',
          'pip',
          'fullscreen',
        ],
        settings: ['speed'],
        speed: {
          selected: 1,
          options: [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2],
        },
        autoplay: currentSettings.autoplay,
        muted: currentSettings.muted,
        volume: currentSettings.volume,
        invertTime: false,
        keyboard: { focused: true, global: true },
        tooltips: { controls: true, seek: true },
        clickToPlay: false, // single/double-click handled manually (see effect)
        // See note in the HLS.js path above — force CSS-only fullscreen so we
        // can promote it to a real OS window fullscreen via Tauri.
        fullscreen: { enabled: true, fallback: 'force', iosNative: false },
        // Disable Plyr's built-in localStorage - we manage settings via Tauri backend
        storage: { enabled: false },
      });

      playerRef.current = player;
      setTimelineHost(containerRef.current?.querySelector<HTMLElement>('.plyr__progress') ?? null);
      playerRef.current.volume = currentSettings.volume;
      playerRef.current.muted = currentSettings.muted;

      player.on('enterfullscreen', () => syncTauriWindowFullscreen(true));
      player.on('exitfullscreen', () => syncTauriWindowFullscreen(false));

      // Persist volume/muted changes back to settings (Safari path)
      player.on('volumechange', () => {
        if (!playerRef.current) return;
        const newVolume = playerRef.current.volume;
        const newMuted = playerRef.current.muted;

        if (volumeDebounceRef.current) clearTimeout(volumeDebounceRef.current);
        volumeDebounceRef.current = setTimeout(() => {
          const { settings, updateSettings } = useAppStore.getState();
          const current = settings.video_player;
          if (current.volume !== newVolume || current.muted !== newMuted) {
            updateSettings({
              ...settings,
              video_player: { ...current, volume: newVolume, muted: newMuted },
            });
          }
        }, 300);
      });

      const onSafariLoadedMetadata = () => {
        video.play().catch(e => Logger.debug('Initial play failed:', e));
      };
      
      onLoadedMetadataRef.current = onSafariLoadedMetadata;
      video.addEventListener('loadedmetadata', onSafariLoadedMetadata);

    } else {
      Logger.error('[HLS] HLS is not supported in this browser');
    }
    // Only depend on streamUrl - settings and isAutoSwitching are read from refs inside the callback
    // This prevents player recreation when volume/muted settings change or when isAutoSwitching toggles
  // handleStreamOffline is accessed via ref to keep this callback stable.
  }, [streamUrl]);

  // Freeze the loader the moment a restart begins. The backend (relay + LL
  // origin) stops before the new stream URL arrives, so without this the old
  // hls.js instance keeps polling a dead origin for the ~1-2s of resolve time and
  // churns non-fatal errors (fragGap "GAP tag found", empty loads) that count
  // toward the auto-switch error budget. The [streamUrl] effect below does the
  // real teardown when the new URL lands; this only stops network activity.
  useEffect(() => {
    if (!isRestartingStream) return;
    try {
      hlsRef.current?.stopLoad();
    } catch {
      /* instance may be mid-teardown */
    }
  }, [isRestartingStream]);

  useEffect(() => {
    const videoElement = videoRef.current;
    if (!videoElement || !streamUrl) return;

    // Start time display update loop
    progressUpdateIntervalRef.current = requestAnimationFrame(updateLiveTimeDisplay);

    const video = videoElement;

    // Deferred-recreate timeout id for the stream-swap path. Held in an
    // effect-scoped variable (not a partial early-return cleanup) so the SINGLE
    // cleanup below always tears everything down — including the rAF loop above.
    let recreateTimeoutId: ReturnType<typeof setTimeout> | null = null;

    // If HLS instance already exists, fully destroy it before creating new one
    // HLS.js doesn't handle reusing instances well after detach/attach cycles
    if (hlsRef.current) {
      Logger.debug('[HLS] Stream URL changed, destroying old HLS instance before creating new one:', streamUrl);

      try {
        // Stop loading new data
        hlsRef.current.stopLoad();
        // Detach from media element first
        hlsRef.current.detachMedia();
        // Then destroy the instance
        hlsRef.current.destroy();
      } catch (e) {
        Logger.warn('[HLS] Error destroying old HLS instance:', e);
      }
      hlsRef.current = null;

      // Thoroughly reset video element to clear any buffered data
      video.pause();
      video.removeAttribute('src');

      // Clear any existing source buffers if MediaSource is attached
      if (video.srcObject) {
        video.srcObject = null;
      }

      video.load();

      // Small delay to ensure video element is fully reset before loading new stream
      // This prevents corrupted TS packets from old stream mixing with new stream
      recreateTimeoutId = setTimeout(() => {
        createPlayer();
      }, 100);
    } else {
      // Create the HLS player (fresh instance for initial load)
      createPlayer();
    }

    // Cleanup
    return () => {
      // Invalidate any createPlayer invocation still awaiting its async probes
      // (it would otherwise construct an orphaned player after this teardown).
      createSeqRef.current++;
      // Clear a pending deferred-recreate so createPlayer can't fire after teardown
      if (recreateTimeoutId) clearTimeout(recreateTimeoutId);
      // Cancel progress update animation frame
      if (progressUpdateIntervalRef.current) {
        cancelAnimationFrame(progressUpdateIntervalRef.current);
        progressUpdateIntervalRef.current = null;
      }

      // Stop the live-latency governor before tearing down the instance.
      if (latencyGovernorStopRef.current) {
        latencyGovernorStopRef.current();
        latencyGovernorStopRef.current = null;
      }

      // Stop the behind-live recovery watchdog.
      if (behindLiveWatchdogRef.current) {
        clearInterval(behindLiveWatchdogRef.current);
        behindLiveWatchdogRef.current = null;
      }

      // Close any active LL diagnostic recording.
      stopLLDiagnostics();

      // Destroy HLS.js instance
      if (hlsRef.current) {
        try {
          hlsRef.current.destroy();
        } catch (e) {
          Logger.warn('Error destroying HLS on cleanup:', e);
        }
        hlsRef.current = null;
      }

      // Destroy Plyr instance
      // CRITICAL: Stop the video decoder BEFORE Plyr.destroy() re-parents the <video> element.
      // React 18 StrictMode double-fires effects (mount → cleanup → mount). If the video is
      // actively decoding when Plyr moves it in the DOM, WebView2's hardware decoder loses its
      // DirectComposition surface — resulting in audio-only playback (black/frozen frame).
      if (playerRef.current) {
        if (videoElement) {
          videoElement.pause();
          videoElement.removeAttribute('src');
          videoElement.load();
        }
        playerRef.current.destroy();
        playerRef.current = null;
        setTimelineHost(null);
      }

      // Reset state
      isLiveRef.current = true;
      userInitiatedPauseRef.current = false;

      // Cleanup DOM listeners and timeouts
      if (videoElement) {
        const playingHandler = onPlayingRef.current;
        const metadataHandler = onLoadedMetadataRef.current;
        const seekingHandler = onSeekingRef.current;
        const timeUpdateHandler = onTimeUpdateRef.current;
        if (playingHandler) videoElement.removeEventListener('playing', playingHandler);
        if (metadataHandler) videoElement.removeEventListener('loadedmetadata', metadataHandler);
        if (seekingHandler) videoElement.removeEventListener('seeking', seekingHandler);
        if (timeUpdateHandler) videoElement.removeEventListener('timeupdate', timeUpdateHandler);
      }
      const debouncer = volumeDebounceRef.current;
      const gateTimeout = bufferGateTimeoutRef.current;
      if (debouncer) clearTimeout(debouncer);
      if (gateTimeout) clearTimeout(gateTimeout);

      // Reset error tracking for next stream
      fatalErrorCountRef.current = 0;
      manifestErrorCountRef.current = 0;
      nonFatalErrorCountRef.current = 0;
      lastSuccessfulPlayRef.current = 0;
      lastErrorTimeRef.current = 0;
      lastFragLoadedTimeRef.current = 0;
    };
  // createPlayer is intentionally excluded: its only dep is streamUrl (already listed).
  // Including it causes double-creation on fresh mounts because React gives useCallback
  // a new identity on the very first render, firing this effect twice.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamUrl, updateLiveTimeDisplay]);

  // Update volume when settings change
  useEffect(() => {
    if (playerRef.current) {
      playerRef.current.volume = playerSettings.volume;
      playerRef.current.muted = playerSettings.muted;
    }
  }, [playerSettings.volume, playerSettings.muted]);

  // Make the Low Latency toggle authoritative on a running stream. The cushion is
  // baked into the hls.js instance at construction (and tightened per-channel after
  // MANIFEST_PARSED), so flipping the toggle mid-stream otherwise does nothing until
  // the next stream change. Rebuilding the player is the only way to apply the new
  // cushion in BOTH directions cleanly: tightening could be eased in by the governor,
  // but loosening (on -> off) cannot without seeking the playhead backward, which is
  // unsafe on a live stream. A restart is consistent with how a quality change
  // already reloads, and toggling latency is an infrequent, deliberate action.
  //
  // We compare against the PREVIOUS value (seeded to the current one) rather than a
  // first-run flag, so neither the initial mount nor React StrictMode's double-invoke
  // nor a remount (which restartStream itself causes) is mistaken for a toggle — only
  // a genuine flip restarts. A first-run flag would let StrictMode's second invoke
  // fire a restart on mount, which then remounts and loops (the toast spam).
  const prevLowLatencyRef = useRef(playerSettings.experimental_low_latency);
  useEffect(() => {
    if (prevLowLatencyRef.current === playerSettings.experimental_low_latency) return;
    prevLowLatencyRef.current = playerSettings.experimental_low_latency;
    // Apply the new mode to the backend origin kill switch FIRST, then restart so the
    // origin probe at stream start honors it (the parts path is gated by this).
    invoke('set_experimental_low_latency', {
      enabled: playerSettings.experimental_low_latency ?? false,
    })
      .catch(() => {})
      .finally(() => {
        if (currentStream && currentMediaType === 'live') {
          Logger.debug('[HLS] Low Latency toggled — restarting stream to apply');
          restartStream();
        }
      });
  }, [playerSettings.experimental_low_latency]); // eslint-disable-line react-hooks/exhaustive-deps

  // Expose an imperative control adapter to the keybinding engine. Methods read
  // playerRef.current at call time so they survive player recreation; isActive()
  // reports false for native-control MP4 clips (no Plyr instance), which keeps
  // player-context hotkeys from firing when there is nothing to control.
  useEffect(() => {
    const SPEEDS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
    const controls: PlayerControls = {
      isActive: () => playerRef.current !== null,
      getCurrentTime: () => {
        const v = videoRef.current;
        return v && Number.isFinite(v.currentTime) ? v.currentTime : null;
      },
      togglePlay: () => playerRef.current?.togglePlay(),
      toggleMute: () => {
        const p = playerRef.current;
        if (p) p.muted = !p.muted;
      },
      toggleFullscreen: () => playerRef.current?.fullscreen.toggle(),
      // Shares stepVolume with the wheel so the hotkey and the mouse can't drift
      // apart on step size or on what happens to mute at the rails.
      volumeUp: () => {
        const p = playerRef.current;
        if (p) stepVolume(p, 1, WHEEL_VOLUME_STEP);
      },
      volumeDown: () => {
        const p = playerRef.current;
        if (p) stepVolume(p, -1, WHEEL_VOLUME_STEP);
      },
      seekForward: () => playerRef.current?.forward(10),
      seekBackward: () => playerRef.current?.rewind(10),
      togglePip: () => {
        const p = playerRef.current;
        if (!p) return;
        try {
          p.pip = !p.pip;
        } catch (err) {
          Logger.debug('[Player] PiP toggle failed:', err);
        }
      },
      speedUp: () => {
        const p = playerRef.current;
        if (!p) return;
        const i = SPEEDS.indexOf(p.speed);
        p.speed = SPEEDS[Math.min(SPEEDS.length - 1, (i < 0 ? 3 : i) + 1)];
      },
      speedDown: () => {
        const p = playerRef.current;
        if (!p) return;
        const i = SPEEDS.indexOf(p.speed);
        p.speed = SPEEDS[Math.max(0, (i < 0 ? 3 : i) - 1)];
      },
    };
    registerPlayerControls(controls);
    return () => registerPlayerControls(null);
  }, []);

  // Click vs double-click on the video surface (live/VOD Plyr path only).
  // Single click toggles play/pause; double click toggles fullscreen. A short
  // timer holds the single-click action just long enough that a following
  // double-click can cancel it, so a double-click never blips play/pause (which
  // on a live stream would force a rebuffer). Listeners live on the <video>
  // node itself, so clicks on the controls bar and the overlay buttons are
  // unaffected. Clip playback uses native controls (no Plyr); there playerRef
  // is null and these handlers no-op, leaving native behavior intact.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    let clickTimer: ReturnType<typeof setTimeout> | null = null;
    const onClick = () => {
      if (clickTimer) return; // second click of a pair — let dblclick handle it
      clickTimer = setTimeout(() => {
        clickTimer = null;
        playerRef.current?.togglePlay();
      }, 250);
    };
    const onDblClick = () => {
      if (clickTimer) {
        clearTimeout(clickTimer);
        clickTimer = null;
      }
      playerRef.current?.fullscreen.toggle();
    };
    video.addEventListener('click', onClick);
    video.addEventListener('dblclick', onDblClick);
    return () => {
      if (clickTimer) clearTimeout(clickTimer);
      video.removeEventListener('click', onClick);
      video.removeEventListener('dblclick', onDblClick);
    };
  }, []);

  // Mouse volume: wheel to change it, middle click to mute. One-handed control
  // for people who don't want to reach for the keyboard.
  //
  // Listeners sit on the player container and nowhere else, so chat, the
  // sidebar, settings and every menu keep their own scrolling — they aren't in
  // this subtree at all. Inside the subtree, ignoresPlayerMouse steps aside for
  // Plyr's menus and its volume slider (which runs its own wheel handler).
  //
  // Registered natively with passive: false because React's onWheel is passive
  // at the root, where preventDefault does nothing. Settings are read from a ref
  // so this stays mounted for the component's life, like the click handler above.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // Plyr for live/VOD; the raw element for clips, which use native controls
    // and have no Plyr instance.
    const volumeTarget = () => playerRef.current ?? videoRef.current;

    const onWheel = (e: WheelEvent) => {
      const s = playerSettingsRef.current;
      // Shift claims the wheel for the channel About reveal whenever both
      // gestures are enabled. Return without consuming so the event bubbles up
      // to ChannelAboutReveal, which owns that half.
      if (e.shiftKey && aboutRevealNeedsShift(s)) return;
      if (!scrollVolumeOn(s)) return;
      // A purely horizontal scroll isn't ours. Chromium turns Shift + wheel into
      // one, so this also keeps a shift-scroll from being swallowed to no effect
      // when the About reveal is switched off.
      if (e.deltaY === 0) return;
      if (ignoresPlayerMouse(e.target)) return;
      const target = volumeTarget();
      if (!target) return;

      // Consume the event even when the accumulator hasn't reached a whole step
      // yet: a sub-notch trackpad nudge must not fall through to the channel
      // About reveal listening on the wrapper above us.
      e.preventDefault();
      e.stopPropagation();

      const steps = wheelAccumRef.current(e, performance.now());
      if (steps === 0) return;
      const next = stepVolume(target, steps, s?.wheel_volume_step ?? WHEEL_VOLUME_STEP);
      showOsd(next.volume, next.muted);
    };

    // Suppress the middle-click autoscroll ring before it starts. Preventing the
    // default here does not stop the auxclick that follows.
    const onMouseDown = (e: MouseEvent) => {
      if (e.button !== 1) return;
      if (!(playerSettingsRef.current?.middle_click_mute ?? true)) return;
      if (ignoresPlayerMouse(e.target)) return;
      e.preventDefault();
    };

    const onAuxClick = (e: MouseEvent) => {
      if (e.button !== 1) return;
      if (!(playerSettingsRef.current?.middle_click_mute ?? true)) return;
      if (ignoresPlayerMouse(e.target)) return;
      const target = volumeTarget();
      if (!target) return;
      e.preventDefault();
      e.stopPropagation();
      const next = toggleVolumeMute(target);
      showOsd(next.volume, next.muted);
    };

    container.addEventListener('wheel', onWheel, { passive: false });
    container.addEventListener('mousedown', onMouseDown, { capture: true });
    container.addEventListener('auxclick', onAuxClick);
    return () => {
      container.removeEventListener('wheel', onWheel);
      container.removeEventListener('mousedown', onMouseDown, { capture: true });
      container.removeEventListener('auxclick', onAuxClick);
    };
  }, [showOsd]);

  // Build the quality menu once BOTH the quality list and the player are ready,
  // in whichever order they arrive. `playerReady` is state, so this re-runs when
  // Plyr is created (a ref wouldn't), and the list can now arrive instantly.
  useEffect(() => {
    if (availableQualities.length > 0 && playerReady) {
      updateQualityMenu();
    }
  }, [availableQualities, playerReady, updateQualityMenu]);

  // Inject a "Stats" toggle into the Plyr settings (gear) menu, alongside Quality
  // and Speed, so the latency panel is opened from there (not a floating button).
  // Re-runs whenever Plyr is (re)created. Mirrors the quality-menu injection: the
  // menu DOM can appear after this fires, so retry until it exists.
  useEffect(() => {
    if (!playerReady) return;
    const container = containerRef.current;
    if (!container) return;
    let attempts = 0;
    let cancelled = false;
    const inject = () => {
      if (cancelled) return;
      const settingsHome = container.querySelector('.plyr__menu [role="menu"]');
      if (!settingsHome) {
        if (attempts++ < 25) setTimeout(inject, 200);
        return;
      }
      if (settingsHome.querySelector('[data-streamnook-stats]')) return; // already present
      const item = document.createElement('button');
      item.className = 'plyr__control';
      item.setAttribute('data-streamnook-stats', '');
      item.setAttribute('type', 'button');
      item.setAttribute('role', 'menuitem');
      item.innerHTML = `<span>Stats<span class="plyr__menu__value" data-stats-value>${showStatsRef.current ? 'On' : 'Off'}</span></span>`;
      item.addEventListener('click', () => setShowStats((s) => !s));
      settingsHome.appendChild(item);
    };
    inject();
    return () => {
      cancelled = true;
    };
  }, [playerReady]);

  // Keep the menu item's On/Off value in sync however showStats changes (the menu
  // item or the panel's own close button).
  useEffect(() => {
    const span = containerRef.current?.querySelector('[data-streamnook-stats] [data-stats-value]');
    if (span) span.textContent = showStats ? 'On' : 'Off';
  }, [showStats]);

  // Start overlay hide timer - use ref to avoid dependency issues
  const startOverlayHideTimer = useCallback(() => {
    if (overlayTimerRef.current) {
      clearTimeout(overlayTimerRef.current);
    }
    overlayTimerRef.current = setTimeout(() => {
      setShowOverlay(false);
    }, OVERLAY_HIDE_DELAY);
  }, []);

  // Snap the playhead back to the live edge (used by the stats panel's "Go Live"
  // button and after scrubbing into the DVR window). Never seek to an unbuffered
  // position: a live hls.js stream parked beyond its buffer freezes, and
  // hls.liveSyncPosition can sit AHEAD of anything downloadable when the relay
  // promotes prefetch hints (the declared edge includes still-encoding segments).
  // So the jump is clamped inside the buffered range with ~2s of forward buffer
  // left to play on. If even the buffered frontier is far behind the sync position
  // the DOWNLOAD itself fell behind (e.g. after system sleep); no seek can fix
  // that, so restart the stream, which cold-starts at the edge. The threshold is 5
  // because a healthy promotion playlist legitimately declares ~2-4s the player
  // cannot fetch yet; only a gap beyond that means the pipeline is actually stuck.
  // VOD position checkpoints to Rust, the owner of the resume store. Null
  // (live, clips, idle) attaches nothing.
  useVodProgressReporter(
    videoRef,
    vodPlayback
      ? {
          videoId: vodPlayback.video_id,
          channelLogin: vodPlayback.channel_login ?? currentStream?.user_login,
          title: vodPlayback.title ?? currentStream?.title,
          thumbnailUrl: vodPlayback.thumbnail_url ?? currentStream?.thumbnail_url,
        }
      : null,
  );
  // One toast when a VOD reopens where the viewer left off.
  useEffect(() => {
    if (!vodPlayback || vodPlayback.rewound_from_live) return;
    const at = vodPlayback.start_position_secs;
    if (!at || at < 1) return;
    if (resumeToastedRef.current === vodPlayback.video_id) return;
    resumeToastedRef.current = vodPlayback.video_id;
    useAppStore.getState().addToast(`Resumed from ${formatVodTime(at)}`, 'info');
  }, [vodPlayback]);
  const rewindLive = useAppStore((s) => s.rewindLive);
  const returnToLive = useAppStore((s) => s.returnToLive);
  // Which delivery path the current hls.js instance rides; the broadcast
  // timeline uses it to decide how far a live seek can go before it must
  // hand off to the recording. State, not a ref, because it is read in
  // render (a ref read there is what the React Compiler refuses).
  const [isLowLatencyPath, setIsLowLatencyPath] = useState(false);
  // Plyr's `.plyr__progress` element, captured right after Plyr builds its
  // controls and cleared when it is destroyed; the timeline portals into it.
  const [timelineHost, setTimelineHost] = useState<HTMLElement | null>(null);
  // One "Resumed from" toast per VOD: StrictMode runs mount effects twice.
  const resumeToastedRef = useRef<string | null>(null);
  const timelineAnchor = liveRewindAnchor ?? currentStream?.started_at ?? null;

  const goLive = useCallback(() => {
    const hls = hlsRef.current;
    const video = videoRef.current;
    if (!hls || !video) return;
    const b = video.buffered;
    const bufferedEnd = b.length > 0 ? b.end(b.length - 1) : 0;
    const pos = hls.liveSyncPosition;
    const syncPos = pos != null && Number.isFinite(pos) ? pos : null;
    if (bufferedEnd === 0 || (syncPos != null && syncPos - bufferedEnd > 5)) {
      restartStream();
      return;
    }
    const target = Math.min(syncPos ?? Infinity, bufferedEnd - 2);
    if (Number.isFinite(target) && target > video.currentTime + 0.25) {
      video.currentTime = target;
    }
    if (video.paused) video.play().catch(() => { /* autoplay policy / teardown */ });
  }, [restartStream]);

  // Mirror the overlay state to the store only while fullscreen, so the
  // fullscreen chat column can fade with the controls. Windowed hover never
  // writes the store.
  useEffect(() => {
    if (!useAppStore.getState().isPlayerFullscreen) return;
    useAppStore.setState({ playerOverlayVisible: showOverlay });
  }, [showOverlay]);

  // Handle mouse events for overlay visibility (works in both normal and fullscreen modes)
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const handleMouseMove = () => {
      // Show overlay on mouse movement
      setShowOverlay(true);
      // Reset the hide timer
      startOverlayHideTimer();
    };

    const handleMouseEnter = () => {
      setShowOverlay(true);
      startOverlayHideTimer();
    };

    const handleMouseLeave = () => {
      // Hide immediately when mouse leaves (unless timer is still running)
      // Clear any existing timer and hide
      if (overlayTimerRef.current) {
        clearTimeout(overlayTimerRef.current);
        overlayTimerRef.current = null;
      }
      setShowOverlay(false);
    };

    container.addEventListener('mousemove', handleMouseMove);
    container.addEventListener('mouseenter', handleMouseEnter);
    container.addEventListener('mouseleave', handleMouseLeave);

    return () => {
      container.removeEventListener('mousemove', handleMouseMove);
      container.removeEventListener('mouseenter', handleMouseEnter);
      container.removeEventListener('mouseleave', handleMouseLeave);
      if (overlayTimerRef.current) {
        clearTimeout(overlayTimerRef.current);
        overlayTimerRef.current = null;
      }
    };
  }, [startOverlayHideTimer]);

  // NOTE: PIP exit handling is done in App.tsx, not here
  // App.tsx correctly differentiates between "Back to tab" (returns to stream view)
  // and "X" button (stops stream) by checking if video is paused

  // Move the current single stream into MultiNook: add it as a tile, switch into
  // the grid, then close the solo player. Lets the viewer pivot from watching one
  // stream to watching several side-by-side without routing back through Home.
  const handleAddToMultiNook = useCallback(async () => {
    const stream = currentStream;
    if (!stream?.user_login) return;

    const login = stream.user_login;
    const mn = usemultiNookStore.getState();
    const provider = streamProvider(stream);
    const addKey = makeKey(provider, login);
    const alreadyPresent = mn.slots.some(
      (s) => makeKey(s.provider ?? 'twitch', s.channelLogin) === addKey
    );

    // MultiNook holds at most 25 tiles. addSlot enforces this too (with its own
    // toast), but checking first avoids closing the current stream when there's
    // no room left to add it.
    if (!alreadyPresent && mn.slots.length >= 25) {
      useAppStore.getState().addToast('Maximum of 25 streams reached', 'warning');
      return;
    }

    // Await the add so slots is non-empty before we toggle. Otherwise
    // toggleMultiNook treats this as an empty entry and reloads the stored
    // lineup, dropping the channel we just added.
    await mn.addSlot(login, provider);

    // Focus MultiNook chat on the channel we came from, keyed the way every
    // consumer compares it (provider:login). Keeping the selection on this same
    // channel means the chat connection carries straight over instead of
    // churning to another tile.
    usemultiNookStore.getState().setActiveChatChannelId(addKey);

    // Enter the grid BEFORE tearing down the solo stream so the chat hook never
    // sees the channel disappear (MultiNook's active slot is this same channel).
    if (!usemultiNookStore.getState().isMultiNookActive) {
      usemultiNookStore.getState().toggleMultiNook();
    }

    // Stop only the solo video proxy and clear its state. preserveBackend keeps
    // the chat bridge, EventSub and drops alive for MultiNook to inherit, and
    // leaves the grid on screen instead of raising Home.
    await exitStream({ preserveBackend: true });
  }, [currentStream, exitStream]);

  // Letterbox fill behind the object-fit video. Cinema Mode keeps it solid black;
  // otherwise it mirrors the title bar exactly (its secondary tint composited over
  // the app base), so the bars and title bar read as one surface and track theme
  // changes automatically since both tokens are theme-driven.
  const letterboxStyle = playerSettings?.cinema_mode
    ? { backgroundColor: '#000', backgroundImage: 'none' }
    : {
        backgroundColor: 'var(--color-background)',
        backgroundImage: 'linear-gradient(var(--color-background-secondary), var(--color-background-secondary))',
      };

  return (
    <div
      ref={containerRef}
      className="w-full h-full flex items-center justify-center video-player-container group"
      style={{
        minHeight: '300px',
        position: 'relative',
        ...letterboxStyle,
      }}
      // Right-clicking the video opens the same stream menu as a tile/sidebar
      // entry (Share, Favorite, Follow, ...), scoped to the channel that's playing.
      onContextMenu={(e) => {
        if (currentStream) {
          useContextMenuStore.getState().openMenu(e, currentStream);
        } else {
          e.preventDefault();
        }
      }}
    >
      <video
        ref={videoRef}
        className="w-full h-full"
        style={{
          width: '100%',
          height: '100%',
          objectFit: 'contain',
          ...letterboxStyle,
        }}
        playsInline
        onLoadedMetadata={(e) => {
          const video = e.target as HTMLVideoElement;
          Logger.debug(`[Video] Metadata loaded: ${video.videoWidth}x${video.videoHeight}`);
          // Safari fallback initial play
          if (!Hls?.isSupported() && video.canPlayType('application/vnd.apple.mpegurl')) {
            video.play().catch(err => Logger.debug('Initial play failed:', err));
          }
        }}
        onPlaying={(e) => {
          const video = e.target as HTMLVideoElement;
          Logger.debug(`[Video] Playing: ${video.videoWidth}x${video.videoHeight}, paused: ${video.paused}, readyState: ${video.readyState}`);
          Logger.debug(`[Video] Audio state: muted=${video.muted}, volume=${video.volume}`);

          // Frames are on screen, so nothing is loading. The overlay is gated on
          // the store's `isLoading` alone, and that flag covers the WHOLE start
          // sequence: the stream url arrives early and the video begins, but the
          // flag stays up through the rest of the session setup (stream info,
          // EventSub, drops), which is why the loading widget could sit on top of
          // a stream that was already playing. It also self-heals any start path
          // that leaves the flag set. This is the one moment the answer is
          // certain, so it is the right place to answer it.
          if (useAppStore.getState().isLoading) {
            useAppStore.setState({ isLoading: false });
          }

          if (lastSuccessfulPlayRef.current === 0) {
            lastSuccessfulPlayRef.current = Date.now();
            Logger.debug('[Video] First successful playback recorded');
          }

          fatalErrorCountRef.current = 0;
          manifestErrorCountRef.current = 0;
          nonFatalErrorCountRef.current = 0;

          // Parity heartbeat gate: minute-watched reporting runs only while
          // the video is actually playing. Cancel any pending paused-report — a
          // resumed playback (incl. an ad-pivot/quality reload finishing) means
          // earning should keep running uninterrupted.
          if (playingPausedTimerRef.current) {
            clearTimeout(playingPausedTimerRef.current);
            playingPausedTimerRef.current = null;
          }
          invoke('report_player_playing', { playing: true }).catch(() => {});
        }}
        onPause={() => {
          // Debounce: don't gate earning off on the transient pause that an
          // ad-bypass region pivot / quality change emits while reloading the
          // source. If playback resumes within the grace window, `onPlaying`
          // cancels this; only a pause that persists actually reports paused.
          if (playingPausedTimerRef.current) {
            clearTimeout(playingPausedTimerRef.current);
          }
          playingPausedTimerRef.current = setTimeout(() => {
            playingPausedTimerRef.current = null;
            invoke('report_player_playing', { playing: false }).catch(() => {});
          }, 6000);
        }}
      />

      {/* Offline Banner (Fallback when no VOD exists) */}
      <AnimatePresence>
        {streamUrl === 'offline' && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="absolute inset-0 z-30 flex flex-col items-center justify-center pointer-events-none"
          >
            {/* Background blur/gradient */}
            <div className="absolute inset-0 bg-gradient-to-br from-purple-900/10 to-black backdrop-blur-3xl" />
            
            {/* Content card */}
            <div className="relative z-10 p-8 rounded-2xl glass-panel border border-white/10 shadow-2xl flex flex-col items-center max-w-md text-center bg-black/40">
              <div className="w-24 h-24 mb-6 rounded-full bg-glass flex items-center justify-center border border-white/20 shadow-[0_0_16px_rgba(145,70,255,0.25)]">
                <span className="text-5xl drop-shadow-lg">😴</span>
              </div>
              <h2 className="text-2xl font-bold text-white mb-3 drop-shadow-md tracking-wide">
                Stream Offline
              </h2>
              <p className="text-white/70 text-sm leading-relaxed mb-8">
                Welcome to the offline chat room. <span className="font-semibold text-white">{currentStream?.user_name || 'The broadcaster'}</span> has no recent videos available to display, but you can still hang out and chat.
              </p>
              
              <div className="flex gap-4 pointer-events-auto">
                <button 
                  onClick={() => {
                    setHomeActiveTab(isAuthenticated ? 'following' : 'recommended');
                    toggleHome();
                  }}
                  className="px-6 py-2.5 glass-button bg-accent/20 border border-accent/40 text-white rounded-lg hover:bg-accent/40 hover:border-accent/60 shadow-[0_0_15px_rgba(145,70,255,0.2)] transition-colors flex items-center gap-2 font-medium"
                >
                  <Home size={16} strokeWidth={2.5} /> Keep Browsing
                </button>
                <button 
                  onClick={async () => {
                    setHomeActiveTab(isAuthenticated ? 'following' : 'recommended');
                    await exitStream();
                  }}
                  className="px-6 py-2.5 glass-button text-white/80 rounded-lg border border-white/10 hover:bg-white/10 hover:text-white transition-colors flex items-center gap-2 font-medium"
                >
                  <XIcon size={16} weight="bold" /> Exit Chat
                </button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Transient stream note — top-left. Replaces the per-stream toasts: how
          this stream is ad-free (your entitlement vs the proxy) plus any quality
          fallback, shown briefly then faded. */}
      <AnimatePresence>
        {showStreamNote && !showOverlay && adSource && currentMediaType === 'live' && (
          <motion.div
            initial={{ opacity: 0, x: -8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -8 }}
            transition={{ duration: 0.3, ease: 'easeInOut' }}
            className="absolute top-4 left-4 z-50 max-w-xs pointer-events-none"
          >
            <div className="glass-panel p-3 rounded-lg border border-accent/30 bg-background/80 backdrop-blur-md">
              <div className="flex items-start gap-2">
                {adSource.entitled ? (
                  <ShieldCheck className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
                ) : adSource.mode === 'auth-only' ? (
                  <ShieldAlert className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
                ) : (
                  <Shield className="w-4 h-4 text-accent flex-shrink-0 mt-0.5" />
                )}
                <div className="space-y-1">
                  <p className="text-textPrimary text-xs font-medium">
                    {adSource.entitled
                      ? adSource.mode === 'turbo'
                        ? 'Ad-free via Twitch Turbo'
                        : 'Ad-free via your subscription'
                      : adSource.mode === 'auth-only'
                        ? 'Direct stream'
                        : 'Ad-free via plugin'}
                  </p>
                  <p className="text-textSecondary text-xs leading-relaxed">
                    {adSource.entitled
                      ? 'Playing directly from Twitch. No proxy.'
                      : adSource.mode === 'auth-only'
                        ? 'Ads may appear on this stream.'
                        : `An installed plugin resolved this stream${adSource.region ? ` through ${adSource.region}` : ''}.`}
                  </p>
                  {activeQuality && !qualitiesEquivalent(settings.quality, activeQuality) && (
                    <p className="text-textMuted text-[10px] leading-relaxed">
                      Playing {activeQuality} ({settings.quality} unavailable).
                    </p>
                  )}
                </div>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Stream Title Overlay — Top-left, shares hover timing with controls */}
      <AnimatePresence>
        {currentStream && showOverlay && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.3, ease: 'easeInOut' }}
            className="stream-title-overlay absolute top-0 left-0 right-0 z-40 pointer-events-none"
          >
          {/* Gradient scrim for text legibility over bright video */}
          <div className="absolute inset-0 bg-gradient-to-b from-black/50 via-black/20 to-transparent" />
          <div className="relative px-4 pt-3 pb-6 flex items-start gap-2">
            {/* Back Arrow or Home Button (Only visible when Home is not active) */}
            {!isHomeActive && (
              currentMediaType === 'clip' || currentMediaType === 'video' ? (
                // Clips/VODs: always show back arrow — stop playback and return to category
                <Tooltip content={streamOriginCategory ? `Back to ${streamOriginCategory.name}` : 'Back'} side="bottom" delay={200}>
                  <button
                    onClick={async () => {
                      if (streamOriginCategory) {
                        setHomeActiveTab('category');
                        setHomeSelectedCategory(streamOriginCategory);
                        // homeCategoryTab is already set to 'clips' or 'videos' from when the card was clicked
                      } else {
                        setHomeActiveTab(isAuthenticated ? 'following' : 'recommended');
                      }
                      await exitStream();
                    }}
                    className="shrink-0 mt-0.5 p-2 glass-button rounded-lg pointer-events-auto"
                    style={{ backdropFilter: 'blur(16px)' }}
                  >
                    <ArrowLeft size={16} weight="bold" className="text-white" />
                  </button>
                </Tooltip>
              ) : streamOriginCategory ? (
                <Tooltip content="Back to category" side="bottom" delay={200}>
                  <button
                    onClick={() => {
                      setHomeActiveTab('category');
                      setHomeSelectedCategory(streamOriginCategory);
                      toggleHome();
                    }}
                    className="shrink-0 mt-0.5 p-2 glass-button rounded-lg pointer-events-auto"
                    style={{ backdropFilter: 'blur(16px)' }}
                  >
                    <ArrowLeft size={16} weight="bold" className="text-white" />
                  </button>
                </Tooltip>
              ) : (
                // No home button here: the title bar owns the Home/Return
                // toggle now, so it does not move between the overlay and the
                // far side of the Home header depending on where you are.
                null
              )
            )}
            <div className="min-w-0">
              {/* Identity row: avatar, display name, partner mark. Mirrors the
                  mobile player's info block so both clients read the same. The
                  live ring is gated on media type — this overlay also serves
                  VODs, clips, and offline chat, where it would be a lie. */}
              <div className="flex items-center gap-2 min-w-0">
                {currentStream.profile_image_url ? (
                  <img
                    src={currentStream.profile_image_url}
                    alt=""
                    draggable={false}
                    className={`w-7 h-7 rounded-full object-cover shrink-0 ${
                      currentMediaType === 'live' ? 'ring-2 ring-live/80' : ''
                    }`}
                  />
                ) : (
                  <div className="w-7 h-7 rounded-full bg-white/15 shrink-0 flex items-center justify-center text-[12px] font-bold text-white">
                    {(currentStream.user_name || currentStream.user_login || '?')
                      .charAt(0)
                      .toUpperCase()}
                  </div>
                )}
                {/* min-w-0: a flex item won't shrink below its content width by
                    default, which makes `truncate` a no-op and overflows instead. */}
                <span className="text-white text-[13.5px] font-semibold truncate min-w-0 drop-shadow-lg">
                  {currentStream.user_name || currentStream.user_login}
                </span>
                {currentStream.broadcaster_type === 'partner' && (
                  <TwitchVerifiedMark size={13} className="text-[#9146FF] shrink-0" />
                )}
              </div>
              {currentStream.title?.trim() && (
                <Tooltip content={currentStream.title} side="bottom" delay={300}>
                <h3
                  className="text-white text-sm font-medium line-clamp-1 drop-shadow-lg mt-1"
                >
                  <StreamTitleWithEmojis title={currentStream.title} />
                </h3>
                </Tooltip>
              )}
              {currentStream.game_name && (
                <p className="text-white/70 text-xs mt-0.5 drop-shadow-md">
                  {currentMediaType === 'live' 
                    ? currentStream.game_name 
                    : streamUrl === 'offline' 
                      ? currentStream.game_name
                      : `${currentStream.game_name} • ${currentStream.viewer_count?.toLocaleString() || 0} Views • ${new Date(currentStream.started_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}`}
                </p>
              )}
            </div>
          </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* The broadcast timeline: spans recording start to the live edge in
          Plyr's progress slot, so a viewer who joined hours in can drag back
          hours. Only when the channel keeps a recording (or we are already in
          it); a VODs-off channel keeps Plyr's session bar. */}
      {isTwitchStream && currentMediaType === 'live' && streamUrl && streamUrl !== 'offline' && timelineAnchor && (liveRewindAvailable || !!liveRewind) && (
        <BroadcastTimeline
          host={timelineHost}
          videoRef={videoRef}
          anchorIso={timelineAnchor}
          rewound={!!liveRewind}
          liveSeekWindowSecs={isLowLatencyPath ? 5 : LIVE_DVR_MAX_BEHIND_SECS}
          visible={showOverlay}
          onSeekLive={(t) => {
            const v = videoRef.current;
            if (v) v.currentTime = t;
          }}
          onGoLive={goLive}
          onRewindTo={(pos) => void rewindLive({ positionSecs: pos })}
          onReturnToLive={() => void returnToLive()}
        />
      )}

      {/* Rewound into the broadcast recording: a persistent way back to the
          live edge (a normal live start on the same channel). */}
      {liveRewind && (
        <button
          onClick={() => void returnToLive()}
          className="absolute left-1/2 top-3 z-30 flex -translate-x-1/2 items-center gap-2 rounded-full glass-button px-3 py-1.5 text-[12px] font-semibold text-white"
          style={{ backdropFilter: 'blur(16px)' }}
        >
          <Radio className="h-3.5 w-3.5 text-red-400" />
          Back to live
        </button>
      )}

      {/* Live telemetry panel, bottom-left. Collapsed toggle rides the hover
          overlay; the panel itself persists once opened. Live streams only. */}
      {currentMediaType === 'live' && !liveRewind && streamUrl && streamUrl !== 'offline' && (
        <PlayerStatsOverlay
          hlsRef={hlsRef}
          videoRef={videoRef}
          open={showStats}
          onToggle={() => setShowStats((s) => !s)}
          onGoLive={goLive}
          adSource={adSource}
        />
      )}

      {/* Follow & Subscribe Button Overlay */}
      <AnimatePresence>
        {currentStream && showOverlay && (currentMediaType === 'live' || currentMediaType === 'offline_chat' || currentMediaType === 'video') && (
          <motion.div
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.3, ease: 'easeInOut' }}
            className="subscribe-overlay absolute top-3 right-3 z-50 flex items-center gap-2"
          >
          {/* Follow Button - Icon Only with Glow */}
          {overlayButtonOn('follow') && (
          <Tooltip content={checkingFollowStatus
                ? 'Checking follow status...'
                : followLoading
                  ? 'Processing...'
                  : isFollowing
                    ? `${terms.unfollow} ${currentStream.user_name}`
                    : `${terms.follow} ${currentStream.user_name}`} side="bottom">
          <button
            onClick={handleFollowClick}
            disabled={followLoading || checkingFollowStatus}
            className={`flex items-center justify-center p-2 glass-button rounded-lg ${followLoading || checkingFollowStatus
              ? 'opacity-60 cursor-wait'
              : ''
              }`}
            style={{ backdropFilter: 'blur(16px)' }}
          >
            {followLoading || checkingFollowStatus ? (
              <Loader2 className="w-4 h-4 animate-spin text-textSecondary" />
            ) : heartDropAnimation ? (
              <HeartBreak
                weight="fill"
                className="w-4 h-4 text-red-400 animate-heart-drop"
              />
            ) : isFollowing ? (
              <HeartBreak
                weight="fill"
                className="w-4 h-4 text-red-400 drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-error)_70%,transparent)]"
              />
            ) : (
              <Heart
                weight="fill"
                className="w-4 h-4 text-emerald-400 drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-success)_70%,transparent)]"
              />
            )}
          </button>
          </Tooltip>
          )}

          {/* Subscribe Button */}
          {/* Hidden when the channel sells no paid tier: most YouTube channels have
              no memberships, and a Join button that leads nowhere is worse than no
              button. Twitch and Kick always report true. */}
          {overlayButtonOn('subscribe') && offersMembership && (
          <Tooltip content={isSubscribed
                ? (membershipTier || membershipDuration
                    // YouTube knows the tier and how long, which is more useful than
                    // a generic gift prompt.
                    ? [membershipTier, membershipDuration].filter(Boolean).join(' · ')
                    : `${terms.paidGift} to ${currentStream.user_name}'s community`)
                : hasSubHistory
                  ? `${terms.paidAgain} to ${currentStream.user_name} (${cumulativeMonths + 1} months)`
                  : `${terms.paid} ${currentStream.user_name}`} side="bottom">
          <button
            onClick={handleSubscribeClick}
            className="flex items-center gap-2 px-4 py-2 glass-button text-white text-sm font-semibold transition-all duration-200"
            style={{ backdropFilter: 'blur(16px)' }}
          >
            <span>
              {isSubscribed ? terms.paidGift : hasSubHistory ? terms.paidAgain : terms.paid}
            </span>
            {subscriberBadgeUrl ? (
              <img 
                src={subscriberBadgeUrl} 
                alt="Subscriber badge" 
                className="w-5 h-5 object-contain"
                referrerPolicy="no-referrer"
              />
            ) : (
              <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                <path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
              </svg>
            )}
          </button>
          </Tooltip>
          )}

          {/* Create Clip Button — live: last ~30s; VOD (incl. offline-chat's
              auto-loaded VOD): ~30s at the current spot */}
          {overlayButtonOn('clip') && canClip && (
            <Tooltip content="Create clip (Alt+X)" side="bottom">
            <button
              onClick={() => createClip()}
              disabled={isCreatingClip}
              className={`flex items-center justify-center p-2 glass-button rounded-lg ${
                isCreatingClip ? 'cursor-wait opacity-70' : ''
              }`}
              style={{ backdropFilter: 'blur(16px)' }}
            >
              {isCreatingClip ? (
                <Loader2 className="w-4 h-4 text-accent animate-spin" />
              ) : (
                <Clapperboard className="w-4 h-4 text-white hover:text-accent transition-colors duration-200" />
              )}
            </button>
            </Tooltip>
          )}

          {/* Identify Song Button — fingerprints a few seconds of player audio
              and names the track (the /song chat command does the same). */}
          {overlayButtonOn('song') && (
            <Tooltip content="Identify song" side="bottom">
            <button
              onClick={() => handleIdentifySong()}
              disabled={isIdentifyingSong}
              className={`flex items-center justify-center p-2 glass-button rounded-lg ${
                isIdentifyingSong ? 'cursor-wait opacity-70' : ''
              }`}
              style={{ backdropFilter: 'blur(16px)' }}
            >
              {isIdentifyingSong ? (
                <Loader2 className="w-4 h-4 text-accent animate-spin" />
              ) : (
                <Music className="w-4 h-4 text-white hover:text-accent transition-colors duration-200" />
              )}
            </button>
            </Tooltip>
          )}

          {/* Share button. Copies the streamnook.app/w/<channel> link; the icon
              pops to a check on copy, and the tooltip confirms. */}
          {overlayButtonOn('share') && currentStream && (
            <Tooltip content={shareCopied ? 'Link copied!' : `Share ${currentStream.user_name}`} side="bottom">
            <button
              onClick={handleSharePlayer}
              className="flex items-center justify-center p-2 glass-button rounded-lg"
              style={{ backdropFilter: 'blur(16px)' }}
            >
              <span key={shareCopied ? 'copied' : 'share'} className="inline-flex animate-in zoom-in-50 duration-200">
                {shareCopied ? (
                  <Check className="w-4 h-4 text-green-400" />
                ) : (
                  <Share2 className="w-4 h-4 text-white hover:text-accent transition-colors duration-200" />
                )}
              </span>
            </button>
            </Tooltip>
          )}

          {/* Clips & VODs — opens this streamer's clip/VOD library. Text label,
              no icon (the label speaks for itself, so no tooltip either).
              Twitch-only: the library is Helix clips + videos, so a provider
              channel id would query an unrelated Twitch broadcaster. */}
          {overlayButtonOn('clipsvods') && isTwitchStream && currentStream && currentStream.user_id && (
            <button
              onClick={() => openStreamerMedia(currentStream)}
              className="flex items-center justify-center px-3 py-2 glass-button rounded-lg text-sm font-semibold text-white hover:text-accent transition-colors duration-200"
              style={{ backdropFilter: 'blur(16px)' }}
            >
              Clips &amp; VODs
            </button>
          )}

          {/* Add to MultiNook Button — pulls this stream into the multi-view grid.
              Twitch-only for the same reason as Clips & VODs above: every grid
              tile resolves a twitch.tv URL, so a provider channel would add a
              tile playing the same-named TWITCH streamer. Worse, this handler
              also exits the current stream, so an ungated click on a Kick or
              YouTube stream tore down what you were watching and handed back a
              dead tile. */}
          {/* Shown for every platform, disabled with the reason on the ones the
              grid cannot run. Hiding it instead reads as a missing feature, which
              is what this looked like while the grid was Twitch-only. */}
          {overlayButtonOn('multinook') && currentMediaType === 'live' && (
            <Tooltip
              content={gridRefusal(streamProvider(currentStream)) ?? 'Add to MultiNook'}
              side="bottom"
            >
            <button
              onClick={handleAddToMultiNook}
              disabled={!canGridProvider(streamProvider(currentStream))}
              className="flex items-center justify-center p-2 glass-button rounded-lg disabled:opacity-40 disabled:cursor-default"
              style={{ backdropFilter: 'blur(16px)' }}
            >
              <LayoutGrid className="w-4 h-4 text-white hover:text-accent transition-colors duration-200" />
            </button>
            </Tooltip>
          )}

          {/* Reload Stream + Chat Button — hard refresh of both the video and
              the chat connection, not just the stream. */}
          {overlayButtonOn('refresh') && currentMediaType === 'live' && (
            <Tooltip content="Reload stream & chat" side="bottom">
            <button
              onClick={async () => {
                setIsRestarting(true);
                try {
                  await reloadStreamAndChat();
                } finally {
                  setIsRestarting(false);
                }
              }}
              disabled={isRestarting}
              className={`flex items-center justify-center p-2 glass-button rounded-lg ${
                isRestarting ? 'cursor-wait opacity-70' : ''
              }`}
              style={{ backdropFilter: 'blur(16px)' }}
            >
              {isRestarting ? (
                <Loader2 className="w-4 h-4 text-accent animate-spin" />
              ) : (
                <RefreshCcw className="w-4 h-4 text-white hover:text-accent transition-colors duration-200" />
              )}
            </button>
            </Tooltip>
          )}

          {/* Close Stream Button */}
          {overlayButtonOn('close') && (
          <Tooltip content="Close Stream" side="bottom">
          <button
            onClick={() => exitStream()}
            className="flex items-center justify-center p-2 glass-button rounded-lg"
            style={{ backgroundColor: 'rgba(239, 68, 68, 0.25)', backdropFilter: 'blur(16px)' }}
          >
            <XIcon weight="bold" className="w-4 h-4 text-red-400" />
          </button>
          </Tooltip>
          )}
          </motion.div>
        )}
      </AnimatePresence>

      {/* Audio Boost popover — edit boost/compressor on the fly without leaving
          the stream. Opened by right-clicking the control-bar toggle. Sits just
          above the control bar; stops pointer events so it never toggles
          play/pause underneath. */}
      <AnimatePresence>
        {audioPanelOpen && (
          <motion.div
            ref={audioPanelRef}
            initial={{ opacity: 0, y: 8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.98 }}
            transition={{ duration: 0.16, ease: 'easeOut' }}
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            onContextMenu={(e) => e.preventDefault()}
            data-no-wheel-volume
            className="liquid-glass-panel absolute bottom-16 right-3 z-[60] rounded-xl p-4"
            style={{ width: 'min(460px, calc(100% - 24px))' }}
          >
            <div className="mb-3 flex items-center justify-between">
              <span className="text-[13px] font-semibold text-textPrimary">Audio Boost</span>
              <Toggle
                enabled={audioBoostEnabled}
                onChange={() => applyBoostPatch({ enabled: !audioBoostEnabled })}
              />
            </div>
            <div className={audioBoostEnabled ? '' : 'opacity-50 pointer-events-none'}>
              <div className="flex flex-wrap items-end justify-center gap-x-5 gap-y-4">
                {audioBoostFaderDefs(resolvedBoost).map((d) => (
                  <Fader
                    key={d.key}
                    label={d.label}
                    display={d.display}
                    value={d.value}
                    min={d.min}
                    max={d.max}
                    step={d.step}
                    hint={d.hint}
                    onChange={(v) => applyBoostPatch(d.apply(v))}
                  />
                ))}
              </div>
            </div>
            <div className="mt-3 flex items-center justify-center gap-4">
              <button
                onClick={() => applyBoostPatch(audioBoostResetPatch())}
                style={{ borderRadius: 8 }}
                className="glass-button text-textSecondary hover:text-textPrimary text-xs px-3 py-1.5"
              >
                Reset to defaults
              </button>
              <button
                onClick={() => {
                  setAudioPanelOpen(false);
                  useAppStore.getState().openSettings('Player', 'settings-section-audio-boost');
                }}
                className="text-xs text-textSecondary underline-offset-2 hover:text-textPrimary hover:underline"
              >
                Open in Settings
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Volume readout for wheel/middle-click changes. globals.css lifts it over
          Plyr's fullscreen element, which is fixed at z-index 10000000. */}
      <PlayerVolumeOsd osd={osd} boost={audioBoostEnabled ? resolvedBoost.gain : 1} />
    </div>
  );
};

export default VideoPlayer;
