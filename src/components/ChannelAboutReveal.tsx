import { useState, useRef, useEffect, type ReactNode } from 'react';
import { platformTerms } from '../utils/platformTerms';
import { streamProvider } from '../utils/streamProvider';
import { motion } from 'framer-motion';
import { ChevronUp, ChevronDown, Heart, HeartCrack, Loader2, Star } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useAppStore } from '../stores/AppStore';
import { aboutRevealNeedsShift, scrollAboutRevealOn } from '../utils/playerMouseControls';
import { playerOverlayButtonOn } from '../utils/playerOverlayButtons';
import { useChannelSocial } from '../hooks/useChannelSocial';
import StreamerAboutPanel from './StreamerAboutPanel';

interface ChannelAboutRevealProps {
  /** Reveal is available (a real stream is playing, not MultiNook). */
  enabled: boolean;
  /** Channel to show About for; also resets the reveal when it changes. */
  channelLogin?: string;
  /** The video player area. */
  children: ReactNode;
}

// A smooth tween (not a spring) for the two-state swap. Both layers run the SAME
// tween, so they stay frame-synced with no overshoot — the seam between video and
// About never flickers. It still reads as a single snap: the wheel/click flips
// the state and this just animates cleanly to it.
const SNAP = { type: 'tween', duration: 0.4, ease: [0.4, 0, 0.2, 1] } as const;
// Minimum wheel delta to flip states — keeps a stray nudge from triggering.
const WHEEL_THRESHOLD = 24;

/**
 * Twitch-style channel reveal. Scrolling down over the player snaps an About
 * drawer up from below, pushing the video up and out of view; scrolling up at
 * the top of the About snaps back to the video. It is a magnetized two-state
 * swap (no partial scroll), and the stream KEEPS PLAYING the whole time — the
 * video element is only translated by a CSS transform, never unmounted.
 */
export default function ChannelAboutReveal({ enabled, channelLogin, children }: ChannelAboutRevealProps) {
  const [showAbout, setShowAbout] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const aboutScrollRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  // True when the player area is taller than the ~16:9 video, so there's a black
  // letterbox bar below it big enough to hold the About hint clear of the stream.
  const [hasBottomBar, setHasBottomBar] = useState(false);
  const currentStream = useAppStore((s) => s.currentStream);
  const playerOverlayButtons = useAppStore((s) => s.settings.player_overlay_buttons);
  // Platform vocabulary: YouTube calls the free relationship "Subscribe" and the
  // paid one "Join", which is the exact inverse of Twitch's wording.
  const terms = platformTerms(streamProvider(currentStream));
  const openStreamerMedia = useAppStore((s) => s.openStreamerMedia);

  // Never reveal over a fullscreen stream. Best-effort via the Tauri window
  // fullscreen flag, refreshed on resize (which fires on fullscreen toggles).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    // Guards the async race: if the effect is torn down before onResized()
    // resolves, the cleanup ran with unlisten still undefined — so unlisten the
    // moment it resolves instead, and never setState after unmount.
    let cancelled = false;
    const w = getCurrentWindow();
    const refresh = () => {
      w.isFullscreen()
        .then((v) => {
          if (!cancelled) setIsFullscreen(v);
        })
        .catch(() => {});
    };
    refresh();
    w.onResized(refresh)
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // The wheel over the player can only mean one thing. By default it changes
  // volume, so the reveal is reached by its hint pill instead; setting the wheel
  // back to "Channel About" restores the scroll-to-reveal gesture.
  // Scroll-to-reveal is its own setting, independent of scroll-to-change-volume.
  // When both are on the plain wheel belongs to volume and this gesture moves to
  // Shift + scroll down, so neither has to be given up for the other.
  const revealOn = useAppStore((s) => scrollAboutRevealOn(s.settings.video_player));
  const revealNeedsShift = useAppStore((s) => aboutRevealNeedsShift(s.settings.video_player));
  // True only when a plain, unmodified scroll down opens the drawer. That's the
  // case the small bottom hint pill was designed for.
  const plainScrollReveals = revealOn && !revealNeedsShift;

  const active = enabled && !!channelLogin && !isFullscreen;
  // Effective open state — never "open" when the reveal isn't active, so a stale
  // true can't push the video off-screen with nothing behind it.
  const open = active && showAbout;

  // Follow / subscribe state for the current channel — only looked up while the
  // About is open (enabled), so it doesn't duplicate the player overlay's checks.
  const {
    isFollowing,
    followLoading,
    checkingFollowStatus,
    handleFollowClick,
    isSubscribed,
    hasSubHistory,
    subscriberBadgeUrl,
    handleSubscribeClick,
    offersMembership,
  } = useChannelSocial({
    userId: currentStream?.user_id,
    userLogin: currentStream?.user_login,
    userName: currentStream?.user_name,
    enabled: open,
  });

  // Reset to the stream when the channel changes (render-phase, React's "adjust
  // state on prop change" pattern — not an effect, so it converges immediately
  // and avoids the synchronous-setState-in-effect cascade).
  const [shownFor, setShownFor] = useState(channelLogin);
  if (channelLogin !== shownFor) {
    setShownFor(channelLogin);
    if (showAbout) setShowAbout(false);
  }

  // Always open the About at the top.
  useEffect(() => {
    if (open && aboutScrollRef.current) aboutScrollRef.current.scrollTop = 0;
  }, [open]);

  // Track whether a bottom letterbox bar exists (player area taller than the
  // ~16:9 video). The hint only shows when the bar can hold it clear of the
  // stream, so it never sits over the actual video content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const measure = () => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (!w || !h) return;
      const bottomBar = (h - (w * 9) / 16) / 2;
      setHasBottomBar(bottomBar >= 30);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const onWheel = (e: React.WheelEvent) => {
    if (!active) return;
    if (!open) {
      // On the video: a downward scroll reveals the About, holding Shift when
      // volume is also on the wheel. The player's own handler mirrors this test
      // and leaves those events alone, so exactly one of us acts.
      if (!revealOn) return;
      if (revealNeedsShift && !e.shiftKey) return;
      // Chromium reroutes Shift + wheel onto the horizontal axis, which can
      // leave deltaY at 0 with deltaX carrying the scroll. Fall back to deltaX
      // only when deltaY is empty, so ordinary vertical scrolling is untouched.
      const delta = e.deltaY !== 0 ? e.deltaY : e.deltaX;
      if (delta > WHEEL_THRESHOLD) setShowAbout(true);
    } else {
      // On the About: it scrolls normally; an upward scroll AT THE TOP returns
      // to the stream.
      const el = aboutScrollRef.current;
      if (el && el.scrollTop <= 0 && e.deltaY < -WHEEL_THRESHOLD) setShowAbout(false);
    }
  };

  return (
    // Stays bound while the About is open regardless of the wheel setting, so
    // scrolling up at the top always returns to the stream.
    <div ref={rootRef} className="group/reveal flex-1 relative overflow-hidden bg-background" onWheel={active && (revealOn || open) ? onWheel : undefined}>
      {/* Video layer — pushed fully up and out when the About is revealed. The
          stream keeps playing; this is only a transform. */}
      <motion.div className="absolute inset-0" animate={{ y: open ? '-100%' : '0%' }} transition={SNAP}>
        {children}
      </motion.div>

      {active && (
        <>
          {/* About drawer — rises from below to fully cover the area. Parked
              off-screen (pointer-events off) until revealed. */}
          <motion.div
            className="absolute inset-0 z-30 flex flex-col bg-background"
            style={{ pointerEvents: open ? 'auto' : 'none' }}
            animate={{ y: open ? '0%' : '100%' }}
            transition={SNAP}
          >
            <div className="flex flex-shrink-0 items-center gap-2 border-b border-borderSubtle bg-background px-4 py-2">
              <button
                type="button"
                onClick={() => setShowAbout(false)}
                className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-semibold text-textSecondary transition-colors hover:bg-glass-hover hover:text-textPrimary"
              >
                <ChevronUp className="h-3.5 w-3.5" />
                Back to stream
              </button>
              {/* Channel actions, mirrored from the player overlay so you can act
                  without leaving the About — including its visibility setting. */}
              <div className="ml-auto flex items-center gap-2">
                {playerOverlayButtonOn(playerOverlayButtons, 'follow') && (
                <button
                  type="button"
                  onClick={handleFollowClick}
                  disabled={followLoading || checkingFollowStatus}
                  className={`flex items-center gap-1.5 px-3 py-1.5 glass-button rounded-lg text-xs font-semibold text-textPrimary ${
                    followLoading || checkingFollowStatus ? 'cursor-wait opacity-60' : ''
                  }`}
                >
                  {followLoading || checkingFollowStatus ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-textSecondary" />
                  ) : isFollowing ? (
                    <HeartCrack className="h-3.5 w-3.5 text-red-400" />
                  ) : (
                    <Heart className="h-3.5 w-3.5 text-emerald-400" />
                  )}
                  {isFollowing ? terms.following : terms.follow}
                </button>
                )}
                {offersMembership && playerOverlayButtonOn(playerOverlayButtons, 'subscribe') && (
                <button
                  type="button"
                  onClick={handleSubscribeClick}
                  className="flex items-center gap-2 px-3 py-1.5 glass-button rounded-lg text-xs font-semibold text-textPrimary"
                >
                  {subscriberBadgeUrl ? (
                    <img src={subscriberBadgeUrl} alt="" className="h-4 w-4 object-contain" referrerPolicy="no-referrer" />
                  ) : (
                    <Star className="h-3.5 w-3.5 text-accent" />
                  )}
                  {isSubscribed ? terms.paidGift : hasSubHistory ? terms.paidAgain : terms.paid}
                </button>
                )}
                {currentStream?.user_id && (
                  <button
                    type="button"
                    onClick={() => currentStream && openStreamerMedia(currentStream)}
                    className="flex items-center px-3 py-1.5 glass-button rounded-lg text-xs font-semibold text-textPrimary"
                  >
                    Clips &amp; VODs
                  </button>
                )}
              </div>
            </div>
            {/* Single scroller: the panel's own overflow is the one that scrolls,
                and we read its scrollTop (aboutScrollRef) to decide when an upward
                scroll at the top should snap back to the stream. */}
            <div className="min-h-0 flex-1">
              {channelLogin && (
                <StreamerAboutPanel channelLogin={channelLogin} scrollRef={aboutScrollRef} />
              )}
            </div>
          </motion.div>

          {/* Scroll affordance, pinned to the bottom edge. Only shown when a bottom
              letterbox bar can hold it (so it never sits over the video). Mounted
              while the bar exists so it FADES (not pops) with the open/hover swap —
              opacity-0 while the About is open, and on hover so it never collides
              with the player's bottom controls (which only appear on hover). */}
          {(hasBottomBar || !plainScrollReveals) && (
            <button
              type="button"
              onClick={() => setShowAbout(true)}
              aria-hidden={open}
              className={`absolute left-1/2 z-20 flex -translate-x-1/2 items-center gap-1 rounded-full border border-transparent bg-black/40 shadow-[inset_1px_1px_0_0_rgba(255,255,255,0.10),inset_-1px_-1px_0_0_rgba(0,0,0,0.18)] px-2.5 py-0.5 text-[11px] font-medium text-white/60 backdrop-blur-sm transition-all duration-300 hover:bg-black/60 hover:text-white ${
                plainScrollReveals
                  // A plain scroll opens the About, so this is only a hint. It
                  // fades out on hover so it never collides with the controls.
                  ? `bottom-2 group-hover/reveal:pointer-events-none group-hover/reveal:opacity-0 ${open ? 'pointer-events-none opacity-0' : 'pointer-events-auto opacity-80'}`
                  // A plain scroll won't open it (Shift-gated, or the gesture is
                  // off), so this pill is the reliable way in. It appears on
                  // hover, above the control bar, where it can be clicked.
                  : `bottom-16 pointer-events-none opacity-0 ${open ? '' : 'group-hover/reveal:pointer-events-auto group-hover/reveal:opacity-80'}`
              }`}
            >
              About
              <ChevronDown className="h-3 w-3" />
            </button>
          )}
        </>
      )}
    </div>
  );
}
