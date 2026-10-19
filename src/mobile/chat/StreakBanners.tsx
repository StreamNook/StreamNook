// Shareable streaks above the composer: a watch-streak milestone, and a resub
// notification with its optional sub-streak.
//
// The desktop banners (WatchStreakBanner / ResubNotificationBanner) are reused
// for their DATA contract but not their chrome — both lean on `group-hover`,
// which does nothing under a thumb, and both are laid out for a wide dock. So
// the same four backend calls drive a compact touch strip instead:
//
//   get_watch_streak      -> milestone for this channel
//   share_watch_streak    -> posts it, consuming the milestone
//   get_resub_notification-> a pending resub token
//   use_resub_token       -> posts it, optionally including the sub streak
//
// Both are one-shot: sharing consumes the token, so each strip disappears
// afterwards rather than lingering as a dead control.
//
// They wear the chat header capsules' glaze (solid at Glassiness 0, as on the
// desktop), a 12 px glyph in the leading column, what it is in bold with the
// detail quieter beside it, then the one glazed accent action and a quiet
// dismiss.
import React, { useCallback, useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Check, Star, X } from 'phosphor-react';
import { WatchStreakFlame } from '../../components/icons/WatchStreakFlame';
import { ACCENT_BUTTON, ACCENT_FILL, BANNER_STYLE, GLAZE_BANNER } from '../../components/ui/glazeButtons';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

interface WatchStreakMilestone {
  milestone_id: string;
  streak_count?: number;
  [k: string]: unknown;
}

interface ResubNotification {
  id: string;
  cumulative_tenure_months: number;
  streak_tenure_months: number;
  months: number;
  is_gift_subscription: boolean;
  gifter_display_name: string | null;
}

interface Props {
  channel: string | null;
  channelId: string | null;
  /** Text currently typed; shared alongside the streak like desktop does. */
  message: string;
  /** Clears the composer once a share consumes the message. */
  onShared: () => void;
}

/** In and out: height and opacity together, on the app's glide curve. */
const FOLD = {
  initial: { height: 0, opacity: 0 },
  animate: { height: 'auto', opacity: 1 },
  exit: { height: 0, opacity: 0 },
  transition: { duration: 0.2, ease: [0.2, 0.8, 0.2, 1] },
} as const;

/** What one room's fetch found, stamped with the room it describes. */
interface Found {
  room: string;
  streak: WatchStreakMilestone | null;
  resub: ResubNotification | null;
}

export const StreakBanners: React.FC<Props> = ({ channel, channelId, message, onShared }) => {
  const addToast = useAppStore((s) => s.addToast);
  // Streaks are per room, and only Twitch rooms have them (a Kick room has no
  // Twitch id). The answer is stamped with its room and compared on read, so
  // switching rooms, landing in a Kick room or a failed fetch can never leave
  // one room's Share chip above another room's chat.
  const room = channel && channelId ? `${channel}:${channelId}` : null;
  const [found, setFound] = useState<Found | null>(null);
  const mine = room && found?.room === room ? found : null;
  const streak = mine?.streak ?? null;
  const resub = mine?.resub ?? null;
  // Dismissals are keyed by channel so leaving and returning does not resurrect
  // something you already waved away, while switching rooms starts clean.
  const [dismissed, setDismissed] = useState<{ channel: string; what: string }[]>([]);
  const [includeStreak, setIncludeStreak] = useState(true);
  const [busy, setBusy] = useState(false);

  const isDismissed = (what: string) =>
    !!channel && dismissed.some((d) => d.channel === channel && d.what === what);

  useEffect(() => {
    if (!room || !channelId) return;
    let cancelled = false;
    // Each half lands on its own, and a failure clears that half rather than
    // leaving whatever was there.
    const take = (patch: Partial<Pick<Found, 'streak' | 'resub'>>) => {
      if (cancelled) return;
      setFound((prev) => ({
        ...(prev?.room === room ? prev : { room, streak: null, resub: null }),
        ...patch,
      }));
    };
    (async () => {
      try {
        const m = await invoke<WatchStreakMilestone | null>('get_watch_streak', { channelId });
        take({ streak: m ?? null });
      } catch (err) {
        Logger.debug('[Streaks] watch streak unavailable:', err);
        take({ streak: null });
      }
      try {
        const r = await invoke<ResubNotification | null>('get_resub_notification', { channelId });
        take({ resub: r ?? null });
      } catch (err) {
        Logger.debug('[Streaks] resub unavailable:', err);
        take({ resub: null });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [room, channelId]);

  const dismiss = useCallback(
    (what: string) => {
      if (!channel) return;
      setDismissed((prev) => [...prev, { channel, what }]);
    },
    [channel],
  );

  // A share consumes the token, so its strip goes; only for the room it was
  // shared from, in case the viewer switched rooms while it was in flight.
  const clear = (sharedRoom: string, what: 'streak' | 'resub') =>
    setFound((prev) => (prev?.room === sharedRoom ? { ...prev, [what]: null } : prev));

  const shareWatchStreak = async () => {
    if (!room || !channelId || !streak || busy) return;
    const sharedRoom = room;
    setBusy(true);
    try {
      const ok = await invoke<boolean>('share_watch_streak', {
        channelId,
        milestoneId: streak.milestone_id,
        message: message.trim() || null,
      });
      if (ok) {
        clear(sharedRoom, 'streak');
        onShared();
      } else {
        addToast('Could not share that watch streak.', 'error');
      }
    } catch (err) {
      Logger.error('[Streaks] watch streak share failed:', err);
      addToast('Could not share that watch streak.', 'error');
    } finally {
      setBusy(false);
    }
  };

  const shareResub = async () => {
    if (!room || !channel || !resub || busy) return;
    const sharedRoom = room;
    setBusy(true);
    try {
      const ok = await invoke<boolean>('use_resub_token', {
        channelLogin: channel,
        message: message.trim() || null,
        includeStreak,
        tokenId: resub.id,
      });
      if (ok) {
        clear(sharedRoom, 'resub');
        onShared();
      } else {
        addToast('Could not share that resub.', 'error');
      }
    } catch (err) {
      Logger.error('[Streaks] resub share failed:', err);
      addToast('Could not share that resub.', 'error');
    } finally {
      setBusy(false);
    }
  };

  const showStreak = !!streak && !isDismissed('watch');
  const showResub = !!resub && !isDismissed('resub');

  // One action, in the theme's accent; the dismiss is a step quieter.
  const share = `${ACCENT_BUTTON} shrink-0 h-7 px-3.5 !text-[12px]`;
  const close =
    'shrink-0 w-7 h-7 flex items-center justify-center rounded-full text-textMuted active:text-textSecondary';
  const surface = `${GLAZE_BANNER} flex items-center gap-2 pl-2.5 pr-1 py-1`;

  return (
    // They rise out of the composer's edge and fold back into it, so the chat
    // above moves once, smoothly, instead of jumping.
    <AnimatePresence initial={false}>
      {/* Resub sits above the watch streak: it is the rarer event and expires,
          so it should be the one you see first. */}
      {showResub && resub && (
        <motion.div key="resub" {...FOLD} className="overflow-hidden">
          <div className="px-2.5 pt-1.5">
            <div className={surface} style={BANNER_STYLE}>
              <Star size={12} weight="fill" className="text-accent shrink-0" />
              <div className="min-w-0 flex-1 py-0.5">
                <div className="text-[12px] truncate">
                  <span className="font-semibold text-textPrimary">
                    {resub.is_gift_subscription ? 'Gift sub' : 'Resub'}
                  </span>
                  <span className="text-textSecondary">
                    {' · '}
                    {resub.is_gift_subscription && resub.gifter_display_name
                      ? `from ${resub.gifter_display_name}`
                      : `${resub.cumulative_tenure_months} month${resub.cumulative_tenure_months === 1 ? '' : 's'}`}
                  </span>
                </div>
                {resub.streak_tenure_months > 1 && (
                  <button
                    onClick={() => setIncludeStreak((v) => !v)}
                    aria-pressed={includeStreak}
                    className="mt-0.5 flex items-center gap-1 text-[11px] text-textMuted active:opacity-70"
                  >
                    <span
                      className={`w-3 h-3 rounded-[3px] border flex items-center justify-center ${
                        includeStreak ? 'bg-accent border-accent' : 'border-textMuted'
                      }`}
                    >
                      {includeStreak && <Check size={8} weight="bold" className="text-background" />}
                    </span>
                    Include the {resub.streak_tenure_months}-month streak
                  </button>
                )}
              </div>
              <button onClick={() => void shareResub()} disabled={busy} className={share} style={ACCENT_FILL}>
                Share
              </button>
              <button onClick={() => dismiss('resub')} className={close} aria-label="Dismiss resub">
                <X size={11} />
              </button>
            </div>
          </div>
        </motion.div>
      )}

      {showStreak && streak && (
        <motion.div key="watch" {...FOLD} className="overflow-hidden">
          <div className="px-2.5 pt-1.5">
            <div className={surface} style={BANNER_STYLE}>
              {/* Twitch's streak flame, in the color the chat's streak rows use. */}
              <WatchStreakFlame size={14} className="shrink-0 text-highlight-orange" />
              <span className="min-w-0 flex-1 py-1 text-[12px] truncate">
                <span className="font-semibold text-textPrimary">Watch streak</span>
                {!!streak.streak_count && (
                  <span className="text-textSecondary">
                    {' · '}
                    {streak.streak_count} stream{streak.streak_count === 1 ? '' : 's'}
                  </span>
                )}
              </span>
              <button onClick={() => void shareWatchStreak()} disabled={busy} className={share} style={ACCENT_FILL}>
                Share
              </button>
              <button onClick={() => dismiss('watch')} className={close} aria-label="Dismiss watch streak">
                <X size={11} />
              </button>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
