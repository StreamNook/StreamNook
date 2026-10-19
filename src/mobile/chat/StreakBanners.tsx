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
// Share arms the banner, as on the desktop: it says "Sharing your watch
// streak" and the composer's next send carries the share (with whatever is
// typed, or nothing). Twitch's answer shows in the banner: a brief "shared",
// then it folds away (sharing consumes the token), or one line saying it did
// not go through, with the banner and the typed text kept for another try.
//
// They wear the chat header capsules' glaze (solid at Glassiness 0, as on the
// desktop), a 12 px glyph in the leading column, what it is in bold with the
// detail quieter beside it, then the one glazed accent action and a quiet
// dismiss.
import React, { useCallback, useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Check, CheckCircle, Star, X } from 'phosphor-react';
import { WatchStreakFlame } from '../../components/icons/WatchStreakFlame';
import { ACCENT_BUTTON, ACCENT_FILL, BANNER_STYLE, GLAZE_BANNER } from '../../components/ui/glazeButtons';
import { invoke } from '@tauri-apps/api/core';
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

/** The armed share, which the composer calls on send with what is typed;
 *  answers whether Twitch took it. */
export type ArmedShare = (text: string) => Promise<boolean>;

interface Props {
  channel: string | null;
  channelId: string | null;
  /** Set while a share is armed, for the composer's send. */
  armedRef: React.MutableRefObject<ArmedShare | null>;
  /** Which share is armed, if any: the composer says so and can send with
   *  nothing typed. */
  onArmedChange: (armed: 'watch' | 'resub' | null) => void;
  focusComposer: () => void;
}

type What = 'watch' | 'resub';

/** Where a share stands, for the room and banner it belongs to. */
interface Step {
  room: string;
  what: What;
  kind: 'armed' | 'sending' | 'shared' | 'failed';
}

/** How long "shared" shows before the banner folds away. */
const SHARED_HOLD_MS = 2500;

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

export const StreakBanners: React.FC<Props> = ({ channel, channelId, armedRef, onArmedChange, focusComposer }) => {
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
  const [step, setStep] = useState<Step | null>(null);
  const stepFor = (what: What) => (step && step.room === room && step.what === what ? step : null);

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
        const r = await invoke<ResubNotification | null>('get_resub_notification', { channelLogin: channel });
        take({ resub: r ?? null });
      } catch (err) {
        Logger.debug('[Streaks] resub unavailable:', err);
        take({ resub: null });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [room, channel, channelId]);

  const dismiss = useCallback(
    (what: string) => {
      if (!channel) return;
      setDismissed((prev) => [...prev, { channel, what }]);
      setStep(null);
    },
    [channel],
  );

  // A share consumes the token, so its banner goes; only for the room it was
  // shared from, in case the viewer switched rooms while it was in flight.
  const clear = (sharedRoom: string, what: What) =>
    setFound((prev) => (prev?.room === sharedRoom ? { ...prev, [what === 'watch' ? 'streak' : 'resub']: null } : prev));

  const arm = (what: What) => {
    if (!room) return;
    setStep({ room, what, kind: 'armed' });
    focusComposer();
  };

  /** Sends the armed share with the composer's text; answers whether Twitch took it. */
  const send = useCallback(
    async (what: What, text: string): Promise<boolean> => {
      if (!room || !channel || !channelId) return false;
      const sharedRoom = room;
      setStep({ room: sharedRoom, what, kind: 'sending' });
      let ok = false;
      try {
        ok =
          what === 'watch'
            ? !!streak &&
              (await invoke<boolean>('share_watch_streak', {
                channelId,
                milestoneId: streak.milestone_id,
                message: text || null,
              }))
            : !!resub &&
              (await invoke<boolean>('use_resub_token', {
                channelLogin: channel,
                message: text || null,
                includeStreak,
                tokenId: resub.id,
              }));
      } catch (err) {
        Logger.warn('[Streaks] share did not go through:', err);
        ok = false;
      }
      if (ok) {
        setStep({ room: sharedRoom, what, kind: 'shared' });
        setTimeout(() => {
          clear(sharedRoom, what);
          setStep((now) => (now?.room === sharedRoom && now.what === what ? null : now));
        }, SHARED_HOLD_MS);
      } else {
        setStep({ room: sharedRoom, what, kind: 'failed' });
      }
      return ok;
    },
    [room, channel, channelId, streak, resub, includeStreak],
  );

  // The composer sends through the armed share until it answers.
  const armed = step && step.room === room && step.kind === 'armed' ? step.what : null;
  useEffect(() => {
    armedRef.current = armed ? (text: string) => send(armed, text) : null;
    onArmedChange(armed);
    return () => {
      armedRef.current = null;
    };
  }, [armed, send, armedRef, onArmedChange]);

  const showStreak = !!streak && !isDismissed('watch');
  const showResub = !!resub && !isDismissed('resub');

  /** Armed or sending: what happens next, and a way out until it is sent. */
  const sharingRow = (icon: React.ReactNode, title: string, s: Step) => (
    <div className={surface} style={BANNER_STYLE} role="status">
      {icon}
      <span className="min-w-0 flex-1 flex flex-col py-0.5">
        <span className="truncate text-[12px] font-semibold text-textPrimary">{title}</span>
        <span className="truncate text-[11px] text-textMuted">
          {s.kind === 'sending' ? 'Sending\u2026' : 'It sends with your next message'}
        </span>
      </span>
      <button
        onClick={() => setStep(null)}
        disabled={s.kind === 'sending'}
        className={`${close} disabled:opacity-40`}
        aria-label="Cancel share"
      >
        <X size={11} />
      </button>
    </div>
  );

  /** Twitch took it: said once, then the banner folds away. */
  const sharedRow = (title: string) => (
    <div className={`${surface} pr-2.5`} style={BANNER_STYLE} role="status">
      <CheckCircle size={14} weight="fill" className="shrink-0 text-success" />
      <span className="min-w-0 flex-1 truncate py-1 text-[12px] font-semibold text-textPrimary">{title}</span>
    </div>
  );

  /** Twitch did not take it: one line, under the banner's own words. */
  const failedLine = (what: What) =>
    stepFor(what)?.kind === 'failed' ? (
      <span role="alert" className="block truncate text-[11px] font-medium text-error">
        Didn't go through. Try again.
      </span>
    ) : null;

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
            {(() => {
              const s = stepFor('resub');
              const title = resub.is_gift_subscription ? 'gift sub' : 'resub';
              if (s?.kind === 'shared') return sharedRow(`Your ${title} is shared`);
              if (s && (s.kind === 'armed' || s.kind === 'sending'))
                return sharingRow(<Star size={12} weight="fill" className="text-accent shrink-0" />, `Sharing your ${title}`, s);
              return (
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
                {failedLine('resub')}
              </div>
              <button onClick={() => arm('resub')} className={share} style={ACCENT_FILL}>
                Share
              </button>
              <button onClick={() => dismiss('resub')} className={close} aria-label="Dismiss resub">
                <X size={11} />
              </button>
            </div>
              );
            })()}
          </div>
        </motion.div>
      )}

      {showStreak && streak && (
        <motion.div key="watch" {...FOLD} className="overflow-hidden">
          <div className="px-2.5 pt-1.5">
            {(() => {
              const s = stepFor('watch');
              // Twitch's streak flame, in the color the chat's streak rows use.
              const flame = <WatchStreakFlame size={14} className="shrink-0 text-highlight-orange" />;
              if (s?.kind === 'shared') return sharedRow('Your watch streak is shared');
              if (s && (s.kind === 'armed' || s.kind === 'sending'))
                return sharingRow(flame, 'Sharing your watch streak', s);
              return (
                <div className={surface} style={BANNER_STYLE}>
                  {flame}
                  <span className="min-w-0 flex-1 flex flex-col py-1">
                    <span className="truncate text-[12px]">
                      <span className="font-semibold text-textPrimary">Watch streak</span>
                      {!!streak.streak_count && (
                        <span className="text-textSecondary">
                          {' · '}
                          {streak.streak_count} stream{streak.streak_count === 1 ? '' : 's'}
                        </span>
                      )}
                    </span>
                    {failedLine('watch')}
                  </span>
                  <button onClick={() => arm('watch')} className={share} style={ACCENT_FILL}>
                    Share
                  </button>
                  <button onClick={() => dismiss('watch')} className={close} aria-label="Dismiss watch streak">
                    <X size={11} />
                  </button>
                </div>
              );
            })()}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
