import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CalendarClock, Loader2, Play, Users } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../stores/AppStore';
import ConfettiBurst from './ConfettiBurst';
import { formatShortCount } from '../utils/streamStats';
import { earnOfflineAccolade, PERFECT_CORNER_ACCOLADE_ID, SHINY_ACCOLADE_ID } from '../utils/offlineAccolades';
import type { OfflineRoom, TwitchStream } from '../types';
import { agoLabel, durationLabel, lastLiveLabel, nextStreamLabel } from '../utils/lastLive';

/** Until a room answers (or for a channel with no room, like a Kick tile):
 *  the same FFZ emotes Rust falls back to. */
const FALLBACK_SAD_EMOTE = 'https://cdn.frankerfacez.com/emote/230082/4';
const FALLBACK_HAPPY_EMOTE = 'https://cdn.frankerfacez.com/emote/228449/4';

/** The drift's timing, in step with globals.css `.offline-bounce-*`: 24 s
 *  across and 15 s down meet at an end together every 120 s (a corner), and
 *  the -7 s start puts the first one 113 s in. */
const DRIFT_START_MS = 7_000;
const CORNER_EVERY_MS = 120_000;
/** How close to a corner the drift must really be when the timer fires (the
 *  window may have been hidden, which pauses the drift but not the timer). */
const CORNER_SLACK_MS = 400;

/** Calls `onCorner` each time the DVD drift lands exactly in a corner. Reads
 *  the running CSS animation's own clock, so it stays honest across pauses;
 *  with no drift (OS reduced motion) it never fires. One timer at a time. */
function useDriftCorners(ref: React.RefObject<HTMLDivElement | null>, enabled: boolean, onCorner: () => void) {
  const fire = useRef(onCorner);
  useEffect(() => {
    fire.current = onCorner;
  });
  useEffect(() => {
    const el = ref.current;
    if (!enabled || !el) return;
    let timer: number | undefined;
    const progress = () => {
      const t = el.getAnimations()[0]?.currentTime;
      return typeof t === 'number' ? t + DRIFT_START_MS : null;
    };
    const schedule = () => {
      const p = progress();
      if (p === null) return;
      const next = Math.floor(p / CORNER_EVERY_MS + 1) * CORNER_EVERY_MS;
      timer = window.setTimeout(() => {
        const now = progress();
        if (now !== null) {
          const off = Math.abs(now - Math.round(now / CORNER_EVERY_MS) * CORNER_EVERY_MS);
          if (off < CORNER_SLACK_MS) fire.current();
        }
        schedule();
      }, next - p + 50);
    };
    schedule();
    return () => window.clearTimeout(timer);
  }, [ref, enabled]);
}

/**
 * Where a channel stands, as a fact rather than a control: a dot and words,
 * the sidebar's language (a hollow ring is offline, a solid red dot live).
 */
export function OfflineStatusLine({ live, facts, className = '' }: { live: boolean; facts: string; className?: string }) {
  return (
    <p className={`flex min-w-0 items-center gap-1.5 text-xs text-textSecondary ${className}`}>
      {live ? (
        <span aria-hidden className="h-[7px] w-[7px] shrink-0 rounded-full bg-live" />
      ) : (
        <span aria-hidden className="h-[7px] w-[7px] shrink-0 rounded-full border-[1.5px] border-textMuted" />
      )}
      {live && <span className="shrink-0 font-medium text-live">Live</span>}
      {facts && (
        <span className="min-w-0 truncate">
          {live && '· '}
          {facts}
        </span>
      )}
    </p>
  );
}

export interface OfflineCardProps {
  room: OfflineRoom | null;
  /** Shown until the room's lookup answers. */
  fallbackName: string;
  fallbackAvatar?: string | null;
  /** The channel is live but not playing here (its playback failed). */
  live: TwitchStream | null;
  joining: boolean;
  /** Plays the latest broadcast in this player; omitted where there is none
   *  to play it in (a MultiNook tile shows the card only). */
  onPlayVod?: () => void;
  vodBusy?: boolean;
  onWatchLive?: (live: TwitchStream) => void;
  /** A MultiNook tile: smaller avatar and padding, no latest broadcast. */
  compact?: boolean;
}

/**
 * A channel that is not streaming, where its stream would be: its offline art
 * letterboxed like the stream, and a card with who it is, when it was last
 * live, and what comes next. Shared by the main view's offline room and an
 * offline MultiNook tile, so offline looks the same in both.
 */
export function OfflineCard({
  room,
  fallbackName,
  fallbackAvatar,
  live,
  joining,
  onPlayVod,
  vodBusy,
  onWatchLive,
  compact = false,
}: OfflineCardProps) {
  const name = room?.display_name || fallbackName;
  const avatar = room?.avatar_url || fallbackAvatar;
  const vod = onPlayVod ? (room?.latest_vod ?? null) : null;
  const next = room?.next_stream ?? null;
  const image = room?.offline_image_url;
  const state: 'offline' | 'live' | 'joining' = joining ? 'joining' : live ? 'live' : 'offline';
  const facts =
    state === 'offline'
      ? [lastLiveLabel(room?.last_live_at), room?.last_category].filter(Boolean).join(' · ')
      : [live?.game_name, live?.title].filter(Boolean).join(' · ');
  const face = compact ? 'h-9 w-9' : 'h-11 w-11';
  // The bounce box takes the emote's own shape (7TV's is square, FFZ's
  // wide), so it turns exactly at the player's edges.
  const [moodRatio, setMoodRatio] = useState(1);
  const moodH = compact ? 52 : 88;
  const mood = { w: Math.round(moodH * moodRatio), h: moodH };
  // Waiting is peepoSad; the moment they go live and the stream starts, it is
  // peepoHappy.
  const moodUrl =
    state === 'joining'
      ? room?.happy_emote_url || FALLBACK_HAPPY_EMOTE
      : room?.sad_emote_url || FALLBACK_SAD_EMOTE;
  const drifting = state !== 'live';
  // You are one of the chatters Twitch counts.
  const waiting = Math.max(0, (room?.chatters ?? 0) - 1);
  // A rare golden peepoSad, while it is waiting (not once they go live).
  const shiny = !!room?.shiny && state === 'offline';
  useEffect(() => {
    if (shiny) earnOfflineAccolade(SHINY_ACCOLADE_ID);
  }, [shiny]);
  const driftRef = useRef<HTMLDivElement>(null);
  // A dead-on corner hit, like the old DVD logo: a burst of confetti.
  const [cornerHits, setCornerHits] = useState(0);
  useDriftCorners(driftRef, drifting, () => {
    setCornerHits((n) => n + 1);
    earnOfflineAccolade(PERFECT_CORNER_ACCOLADE_ID);
  });

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.25 }}
      className="absolute inset-0 z-30 flex items-center justify-center overflow-hidden"
    >
      {image ? (
        // Letterboxed like the stream it stands in for: offline art is drawn
        // for a 16:9 player and often has text at its edges, which cropping cut.
        <img src={image} alt="" draggable={false} className="absolute inset-0 h-full w-full bg-black object-contain" />
      ) : (
        <div className="absolute inset-0 bg-gradient-to-br from-accent/10 via-background to-black" />
      )}
      <div className="absolute inset-0 bg-gradient-to-t from-black/60 via-black/30 to-black/15" />

      {/* peepoSad drifting around the player like an old DVD logo, waiting for
          them with you (peepoHappy once they go live): the room's mood, under
          the card and never part of its text. Transform-only CSS
          (globals.css .offline-bounce-*). */}
      {drifting && (
        <div
          ref={driftRef}
          aria-hidden
          className="offline-bounce-x"
          style={{ '--bounce-w': `${mood.w}px`, '--bounce-h': `${mood.h}px` } as CSSProperties}
        >
          <div className="offline-bounce-y">
            <img
              src={moodUrl}
              alt=""
              draggable={false}
              // A static filter is rasterized once with the layer, so the
              // golden one drifts as cheaply as the plain one.
              className={`absolute left-0 top-0 select-none object-contain ${
                shiny ? 'offline-shiny' : 'opacity-90 drop-shadow-[0_2px_6px_rgba(0,0,0,0.55)]'
              }`}
              style={{ width: mood.w, height: mood.h }}
              onLoad={(e) => {
                const { naturalWidth: w, naturalHeight: h } = e.currentTarget;
                if (w > 0 && h > 0) setMoodRatio(w / h);
              }}
            />
          </div>
        </div>
      )}
      {cornerHits > 0 && <ConfettiBurst key={cornerHits} />}

      <div className={`relative z-10 flex max-h-[calc(100%-24px)] w-[calc(100%-32px)] flex-col ${compact ? 'max-w-[320px]' : 'max-w-[460px]'}`}>
      <div
        // A short player scrolls the card inside itself rather than clipping it.
        className={`glass-flyout flex min-h-0 w-full flex-col overflow-y-auto ${compact ? 'gap-3' : 'gap-4'}`}
        style={{ borderRadius: compact ? 14 : 16, padding: compact ? 14 : 20, scrollbarWidth: 'none' }}
      >
        <div className="flex items-center gap-3">
          {avatar ? (
            <img
              src={avatar}
              alt=""
              draggable={false}
              className={`${face} shrink-0 rounded-full object-cover ${state === 'offline' ? '' : 'ring-2 ring-live/80'}`}
            />
          ) : (
            <div className={`${face} flex shrink-0 items-center justify-center rounded-full bg-white/10 text-base font-bold text-white`}>
              {name.charAt(0).toUpperCase()}
            </div>
          )}
          <div className="min-w-0 flex-1">
            <h2 className={`truncate font-semibold text-textPrimary ${compact ? 'text-sm' : 'text-base'}`}>{name}</h2>
            <OfflineStatusLine live={state !== 'offline'} facts={facts} className="mt-0.5" />
          </div>
        </div>

        {state === 'joining' ? (
          <div className="flex items-center gap-2 text-xs text-textSecondary">
            <Loader2 size={14} className="animate-spin" />
            Starting the stream
          </div>
        ) : state === 'live' && live && onWatchLive ? (
          <button
            type="button"
            onClick={() => onWatchLive(live)}
            className="flex h-9 items-center justify-center gap-2 rounded-lg bg-live text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white/60"
          >
            <Play size={14} fill="currentColor" />
            Watch live
          </button>
        ) : null}

        {/* The latest broadcast is one action, not a preview: it plays in the
            player this card sits in. */}
        {state !== 'joining' && vod && onPlayVod && (
          <button
            type="button"
            onClick={onPlayVod}
            disabled={vodBusy}
            title={vod.title || undefined}
            className="glass-button flex h-9 items-center gap-2 rounded-lg px-3 text-left text-xs font-medium text-textPrimary disabled:cursor-wait"
          >
            <Play size={13} fill="currentColor" className="shrink-0" />
            <span className="min-w-0 flex-1 truncate">
              Watch the {vod.category ? `${vod.category} ` : ''}stream from {agoLabel(vod.created_at) ?? 'last time'}
            </span>
            <span className="shrink-0 tabular-nums text-textSecondary">{durationLabel(vod.length_seconds)}</span>
          </button>
        )}

        {/* No "Check again": Rust already watches for the go-live (EventSub at
            once when signed in, the shared minute poll always). */}
        {state === 'offline' && (
          <div className="flex min-w-0 items-center gap-3 text-xs text-textSecondary">
            <p className="flex min-w-0 flex-1 items-center gap-1.5 truncate">
              {next ? (
                <>
                  <CalendarClock size={13} className="shrink-0" />
                  <span className="truncate">
                    Next stream <span className="text-textPrimary">{nextStreamLabel(next.start_at)}</span>
                    {next.category ? ` · ${next.category}` : ''}
                  </span>
                </>
              ) : (
                <span className="truncate">The stream starts here when they go live.</span>
              )}
            </p>
            {/* Waiting buddies: everyone else in the chat right now. */}
            {waiting > 0 && (
              <span className="flex shrink-0 items-center gap-1 tabular-nums" title="In the chat right now, waiting with you">
                <Users size={13} className="shrink-0" />
                {formatShortCount(waiting)} waiting
              </span>
            )}
          </div>
        )}
      </div>
      </div>
    </motion.div>
  );
}

/**
 * The main view while a Twitch channel is offline: the offline card, wired to
 * the store. The session behind it starts the stream when the channel goes
 * live, so the card's last state is "Starting the stream", never a toast.
 */
export default function OfflineRoomScreen() {
  const { room, stream, live, joining, isLoading, playVod, startStream } = useAppStore(
    useShallow((s) => ({
      room: s.offlineRoom,
      stream: s.currentStream,
      live: s.offlineRoomLive,
      joining: s.offlineRoomJoining,
      isLoading: s.isLoading,
      playVod: s.playOfflineRoomVod,
      startStream: s.startStream,
    })),
  );
  return (
    <OfflineCard
      room={room}
      fallbackName={stream?.user_name || stream?.user_login || 'This channel'}
      fallbackAvatar={stream?.profile_image_url}
      live={live}
      joining={joining}
      onPlayVod={() => void playVod()}
      vodBusy={isLoading}
      onWatchLive={(l) => void startStream(l.user_login, l, true)}
    />
  );
}

/**
 * The room's channel went live while its latest broadcast plays in the room:
 * offered, never forced. Renders nothing otherwise, and reads the store
 * itself so the player does not re-render for it.
 */
export function OfflineRoomLivePill() {
  const { live, show, startStream } = useAppStore(
    useShallow((s) => ({
      live: s.offlineRoomLive,
      show: s.currentMediaType === 'offline_chat' && s.streamUrl !== 'offline' && !!s.offlineRoomLive,
      startStream: s.startStream,
    })),
  );
  return (
    <AnimatePresence>
      {show && live && (
        <motion.button
          type="button"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          onClick={() => void startStream(live.user_login, live, true)}
          className="glass-flyout absolute left-1/2 top-3 z-50 flex -translate-x-1/2 items-center gap-2 text-xs font-medium text-textPrimary"
          style={{ borderRadius: 999, padding: '6px 12px' }}
        >
          <span aria-hidden className="h-[7px] w-[7px] shrink-0 rounded-full bg-live" />
          {live.user_name || live.user_login} is live now
          <span className="text-textSecondary">· Watch live</span>
        </motion.button>
      )}
    </AnimatePresence>
  );
}
