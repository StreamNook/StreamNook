import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import { streamProvider, followIdentifier } from '../utils/streamProvider';
import { useFollowsStore } from '../stores/followsStore';
import { useChatConnectionStore } from '../stores/chatConnectionStore';
import { makeKey } from '../utils/providerKey';
import type { ProviderId } from '../types/providers';

interface ChannelSocialTarget {
  /** Which platform this channel is on. Pass it whenever the channel is NOT the
   *  app's current solo stream, which is any MultiNook tile: without it the hook
   *  falls back to `currentStream`'s platform and a Twitch tile can show, and
   *  write, a Kick channel's follow state. Absent means "use currentStream",
   *  preserving the solo player's behaviour exactly. */
  provider?: ProviderId;
  /** Broadcaster id of the channel on `provider`. */
  userId?: string | null;
  /** Login / slug / handle of the channel on `provider`. */
  userLogin?: string | null;
  /** Display name of the channel (for window titles / tooltips). */
  userName?: string | null;
  /** When false the hook stays idle — no follow/subscription lookups fire.
   *  Lets MultiNook run it only for the focused tile instead of every cell. */
  enabled?: boolean;
}

/** Follow + subscribe state and actions for a single channel.
 *
 *  Extracted from the single-stream player so the same controls can back both
 *  the main VideoPlayer overlay and the focused MultiNook tile without
 *  duplicating the follow-status / subscription / subscribe-window logic. */
export function useChannelSocial({ provider: providerProp, userId, userLogin, userName, enabled = true }: ChannelSocialTarget) {
  const currentUser = useAppStore((s) => s.currentUser);

  // Which platform this channel is on, and (for the non-Twitch ones) the app's own
  // follow list, which is what a provider follow reads and writes.
  const currentStream = useAppStore((s) => s.currentStream);
  // An explicit provider always wins: the caller knows which channel this is,
  // where currentStream only describes the solo player.
  const provider = providerProp ?? streamProvider(currentStream);
  const isTwitch = provider === 'twitch';
  const providerFollows = useFollowsStore((s) => s.follows);
  // Keyed by CHANNEL, not broadcast — see `followIdentifier`. Shared with the
  // Home grid's heart so the two surfaces cannot disagree about what "follow
  // this" means; they previously did, and the heart was wrong as a result.
  // With an explicit provider the caller's own login is the identifier; only the
  // currentStream-derived path needs followIdentifier to unpick a broadcast.
  const providerChannel: string =
    !isTwitch && !providerProp && currentStream
      ? followIdentifier(currentStream)
      : (userLogin ?? '');

  // Follow state
  const [isFollowing, setIsFollowing] = useState<boolean | null>(null);
  const [followLoading, setFollowLoading] = useState(false);
  const [checkingFollowStatus, setCheckingFollowStatus] = useState(true);
  const [heartDropAnimation, setHeartDropAnimation] = useState(false);

  // Subscription state
  const [isSubscribed, setIsSubscribed] = useState<boolean>(false);
  const [hasSubHistory, setHasSubHistory] = useState<boolean>(false);
  const [cumulativeMonths, setCumulativeMonths] = useState<number>(0);
  const [subscriberBadgeUrl, setSubscriberBadgeUrl] = useState<string | null>(null);
  // Whether this channel sells the paid tier at all. Twitch/Kick always do; on
  // YouTube most channels have no memberships, so the Join button should not be
  // offered there. Defaults to true so nothing is hidden before the answer lands.
  const [offersMembership, setOffersMembership] = useState<boolean>(true);
  // YouTube membership detail, for the tooltip. Twitch expresses the same thing as
  // cumulative months, which it already has.
  const [membershipTier, setMembershipTier] = useState<string | null>(null);
  const [membershipDuration, setMembershipDuration] = useState<string | null>(null);

  // The channel's member badge, taken from CHAT.
  //
  // Chat already carries the real per-tier art: the adapter reads YouTube's
  // `customThumbnail` off each author's badges, which is why chat renders tiers
  // correctly. Those messages are already flowing over the open connection, so this
  // costs no request at all, and it is the only source that works for someone who
  // is NOT a member (the offers endpoint 400s, and the join page renders its dialog
  // client-side).
  //
  // The LOWEST month count wins: the adapter stores a member badge's tier in
  // `version` as its month count, so the smallest is the badge a new member gets,
  // which is what the Join button should be previewing.
  const chatRevision = useChatConnectionStore((s) => s.revision);
  const chatBadgeRef = useRef<{ key: string; url: string; months: number } | null>(null);
  const badgeTraceRef = useRef<string | null>(null);
  const chatMemberBadge = useMemo(() => {
    // Read for its change signal only: the badge itself comes from the store below,
    // but a new flush is exactly when a new badge can appear. Same idiom as
    // LiveOverlayFeed's `void revision`.
    void chatRevision;
    if (isTwitch || !userLogin) return null;
    const key = makeKey(provider, userLogin);
    // A different channel invalidates whatever was found for the last one.
    if (chatBadgeRef.current && chatBadgeRef.current.key !== key) chatBadgeRef.current = null;
    const slice = useChatConnectionStore.getState().channels.get(key);
    for (const raw of slice?.messages ?? []) {
      const badges = (raw as { badges?: { name?: string; version?: string;
        image_url_1x?: string; image_url_2x?: string; image_url_4x?: string }[] })?.badges;
      if (!Array.isArray(badges)) continue;
      for (const b of badges) {
        if (b?.name !== 'subscriber') continue;
        const url = b.image_url_4x || b.image_url_2x || b.image_url_1x;
        if (!url) continue;
        const months = Number(b.version) || 0;
        const held = chatBadgeRef.current;
        if (!held || months < held.months) chatBadgeRef.current = { key, url, months };
      }
    }
    // Trace ONCE per channel, into the app log. A missing badge has several
    // possible broken links (wrong slice key, no messages yet, no member badge in
    // the buffer), and this names which one rather than leaving it to guesswork.
    // Report on arrival at a channel AND again the moment a badge first turns up.
    // Reporting only once per channel would almost always say "none": the first
    // pass runs before any member has posted, which is the normal case and not the
    // failure being looked for.
    const found = chatBadgeRef.current?.url ?? 'none';
    const stamp = `${key}|${found}|${!!slice}`;
    if (badgeTraceRef.current !== stamp) {
      badgeTraceRef.current = stamp;
      // Sent to the BACKEND log on purpose: the frontend Logger writes to the
      // devtools console and its info level is off by default, so a trace there is
      // invisible in the log file that actually gets read.
      void invoke('log_frontend_diag', {
        message:
          `[badge] key=${key} slice=${!!slice} messages=${slice?.messages?.length ?? 0} ` +
          `found=${found} months=${chatBadgeRef.current?.months ?? '-'}` +
          // When the slice is missing, the keys that DO exist say whether this is a
          // naming mismatch or simply nothing connected yet.
          (slice
            ? ''
            : ` have=[${[...useChatConnectionStore.getState().channels.keys()].join(',')}]`),
      }).catch(() => { /* diagnostics must never break the render path */ });
    }
    return chatBadgeRef.current?.url ?? null;
    // `chatRevision` ticks on every chat flush, which is exactly when a new badge
    // can appear. The scan is over the capped message buffer, so it stays cheap.
  }, [isTwitch, provider, userLogin, chatRevision]);

  // Check follow status when the channel changes
  useEffect(() => {
    if (!enabled) {
      setIsFollowing(null);
      setCheckingFollowStatus(false);
      return;
    }
    // Non-Twitch: the answer is already in the app's follow list, so there is
    // nothing to ask for. `check_following_status` is Helix and would be answering
    // about the wrong platform entirely.
    if (!isTwitch) {
      setIsFollowing(
        !!providerChannel && useFollowsStore.getState().isFollowed(provider, providerChannel),
      );
      setCheckingFollowStatus(false);
      return;
    }
    if (!userId) {
      setIsFollowing(null);
      setCheckingFollowStatus(false);
      return;
    }

    const checkFollowStatus = async () => {
      try {
        setCheckingFollowStatus(true);
        const result = await invoke<boolean>('check_following_status', { targetUserId: userId });
        setIsFollowing(result);
      } catch (err) {
        Logger.error('[useChannelSocial] Failed to check follow status:', err);
        setIsFollowing(false);
      } finally {
        setCheckingFollowStatus(false);
      }
    };

    checkFollowStatus();
    // `providerFollows` is a dep so the state re-derives when the list changes
    // (an import, or a follow made from another surface).
  }, [enabled, userId, isTwitch, provider, providerChannel, providerFollows]);

  // Check subscription status when the channel changes
  useEffect(() => {
    if (!enabled) {
      setIsSubscribed(false);
      setHasSubHistory(false);
      setCumulativeMonths(0);
      setSubscriberBadgeUrl(null);
      setOffersMembership(true);
      setMembershipTier(null);
      setMembershipDuration(null);
      return;
    }
    // Kick: the account sync already imports which channels the user subscribes
    // to, so the answer is in the follow list. No lookup, and it stays right
    // through a re-sync.
    if (provider === 'kick') {
      setIsSubscribed(
        !!providerChannel && useFollowsStore.getState().isSubscribed(provider, providerChannel),
      );
      setHasSubHistory(false);
      setCumulativeMonths(0);
      setSubscriberBadgeUrl(null);
      setOffersMembership(true);
      return;
    }
    // YouTube: memberships are per channel and many channels sell none, so ask.
    if (provider === 'youtube') {
      setHasSubHistory(false);
      setCumulativeMonths(0);
      setSubscriberBadgeUrl(null);
      if (!providerChannel) {
        setIsSubscribed(false);
        setOffersMembership(false);
        return;
      }
      let cancelled = false;
      void invoke<{
        offers: boolean;
        is_member: boolean;
        badge_url?: string | null;
        tier?: string | null;
        duration?: string | null;
      }>('provider_membership', { provider, channel: providerChannel })
        .then((state) => {
          if (cancelled) return;
          setIsSubscribed(!!state?.is_member);
          setOffersMembership(!!state?.offers);
          // The member's own badge art, which is what Twitch's subscriberBadgeUrl
          // holds, so the same control renders it with no extra branching.
          setSubscriberBadgeUrl(state?.badge_url || null);
          setMembershipTier(state?.tier || null);
          setMembershipDuration(state?.duration || null);
        })
        .catch(() => {
          if (cancelled) return;
          setIsSubscribed(false);
          // Unknown is treated as "offers", so a lookup failure never hides a
          // control the channel may genuinely have.
          setOffersMembership(true);
        });
      return () => {
        cancelled = true;
      };
    }
    if (!userId || !userLogin || !currentUser?.login) {
      setIsSubscribed(false);
      setHasSubHistory(false);
      setCumulativeMonths(0);
      setSubscriberBadgeUrl(null);
      return;
    }

    const channelId = userId;
    const channelLogin = userLogin;
    const loginOfUser = currentUser.login;

    const checkSubscriptionStatus = async () => {
      try {
        const { fetchIVRSubage } = await import('../services/ivrService');
        const subageData = await fetchIVRSubage(loginOfUser, channelLogin);

        // IVR API uses meta.type to indicate an active sub ("paid", "gift", "prime", etc.)
        const metaData = (subageData as unknown as Record<string, unknown>)?.meta as Record<string, unknown> | undefined;
        const isSub = metaData?.type != null;
        const cumMonths = subageData?.cumulative?.months ?? 0;

        setIsSubscribed(isSub);
        setHasSubHistory(cumMonths > 0 && !isSub);
        setCumulativeMonths(cumMonths);

        // Determine which badge version to show
        let badgeMonths = cumMonths;
        if (!isSub && cumMonths > 0) {
          // Lapsed subscriber: show badge for the NEXT month they'd reach
          badgeMonths = cumMonths + 1;
        }

        const getBadgeVersion = (months: number): string => {
          if (months >= 72) return '72';
          if (months >= 60) return '60';
          if (months >= 48) return '48';
          if (months >= 36) return '36';
          if (months >= 24) return '24';
          if (months >= 18) return '18';
          if (months >= 12) return '12';
          if (months >= 9) return '9';
          if (months >= 6) return '6';
          if (months >= 3) return '3';
          if (months >= 2) return '2';
          return '0';
        };

        const badgeVersion = getBadgeVersion(badgeMonths);

        const { initializeBadgeCache, parseBadges } = await import('../services/twitchBadges');
        await initializeBadgeCache(channelId);
        const badges = parseBadges(`subscriber/${badgeVersion}`, channelId);

        if (badges.length > 0 && badges[0].info?.image_url_2x) {
          setSubscriberBadgeUrl(badges[0].info.image_url_2x);
        } else {
          setSubscriberBadgeUrl(null);
        }
      } catch (err) {
        Logger.error('[useChannelSocial] Failed to check subscription status:', err);
        setIsSubscribed(false);
        setHasSubHistory(false);
        setSubscriberBadgeUrl(null);
      }
    };

    checkSubscriptionStatus();
  }, [enabled, userId, userLogin, currentUser?.login, provider, providerChannel, providerFollows]);

  // Handle follow/unfollow action
  const handleFollowClick = useCallback(async () => {
    if (followLoading || (isTwitch ? !userId : !providerChannel)) return;

    const action = isFollowing ? 'unfollow' : 'follow';

    // If unfollowing, play the heart-drop animation first
    if (isFollowing) {
      setHeartDropAnimation(true);
      await new Promise((resolve) => setTimeout(resolve, 600));
      setHeartDropAnimation(false);
    }

    setFollowLoading(true);
    Logger.debug(`[useChannelSocial] Initiating ${action} for ${userLogin} (ID: ${userId})`);

    try {
      if (isTwitch) {
        const command = isFollowing ? 'unfollow_channel' : 'follow_channel';
        await invoke(command, { targetUserId: userId });
      } else {
        // Non-Twitch: the app's own follow list, which for a signed-in YouTube
        // account also subscribes on YouTube itself. `follow_channel` is Helix, so
        // sending a YouTube channel to it was never going to work.
        const store = useFollowsStore.getState();
        if (isFollowing) await store.unfollow(provider, providerChannel);
        else await store.follow(provider, providerChannel, userName ?? undefined);
      }
      const nowFollowing = !isFollowing;
      setIsFollowing(nowFollowing);
      // Announce it, so anything else keyed on follow state updates without
      // making its own request. The mobile composer blocks on "Follow to send a
      // message" in followers-only rooms and listens for this, so following
      // unlocks it immediately instead of at the next remount.
      window.dispatchEvent(
        new CustomEvent('sn:follow-changed', {
          detail: { userId, following: nowFollowing },
        }),
      );
      Logger.debug(`[useChannelSocial] Successfully ${action}ed ${userLogin}`);
    } catch (err) {
      Logger.error(`[useChannelSocial] ${action} error:`, err);
      useAppStore.getState().addToast(
        `Follow/Unfollow failed. Try logging out and back in via Settings to re-authenticate.`,
        'error'
      );
    } finally {
      setFollowLoading(false);
    }
  }, [userLogin, userId, isFollowing, followLoading, isTwitch, provider, providerChannel, userName]);

  // Track the subscribe window's label so we can auto-close it on a successful sub
  const subscribeWindowLabelRef = useRef<string | null>(null);

  // Listen for subscription events to auto-close the subscribe window
  useEffect(() => {
    if (!enabled) return;

    const handleSubscriptionDetected = async (event: Event) => {
      const customEvent = event as CustomEvent<{ login: string; msgId: string; displayName: string }>;
      const { login, msgId } = customEvent.detail;
      const currentUserLogin = currentUser?.login?.toLowerCase();

      // Only react to the current user's own subscription on this channel's window
      if (currentUserLogin && login === currentUserLogin && subscribeWindowLabelRef.current) {
        useAppStore.getState().addToast(
          `Subscription successful! ${msgId === 'subgift' ? 'Gift sent!' : 'Thank you for subscribing!'}`,
          'success'
        );

        try {
          await invoke('close_login_overlay', { label: subscribeWindowLabelRef.current });
        } catch (e) {
          Logger.warn('[useChannelSocial] Failed to close subscribe overlay:', e);
        }

        subscribeWindowLabelRef.current = null;
      }
    };

    window.addEventListener('twitch-subscription-detected', handleSubscriptionDetected);
    return () => {
      window.removeEventListener('twitch-subscription-detected', handleSubscriptionDetected);
    };
  }, [enabled, currentUser?.login]);

  // Open the Twitch subscribe page for this channel in a dedicated window,
  // isolated to the active (main) account's Twitch web profile so you subscribe
  // as the account you watch and stream as. The backend returns the window label
  // so the subscription listener above can auto-close it.
  const handleSubscribeClick = useCallback(async () => {
    if (!userLogin) return;
    try {
      const label = await invoke<string>('open_subscribe_window', {
        channelLogin: userLogin,
        title: `Subscribe to ${userName || userLogin}`,
        // Each platform has its own subscribe page; the backend picks the URL
        // and the signed-in web profile to open it in.
        provider: streamProvider(useAppStore.getState().currentStream),
      });
      subscribeWindowLabelRef.current = label;
    } catch (e) {
      Logger.error('[useChannelSocial] Error opening subscribe window:', e);
      subscribeWindowLabelRef.current = null;
    }
  }, [userLogin, userName]);

  return {
    // Follow
    isFollowing,
    followLoading,
    checkingFollowStatus,
    heartDropAnimation,
    handleFollowClick,
    // Subscribe
    isSubscribed,
    // A member's OWN badge when we have it, else the channel's base-tier badge
    // seen in chat, so the control is never a bare placeholder on a channel that
    // clearly has member badges.
    subscriberBadgeUrl: subscriberBadgeUrl || chatMemberBadge,
    offersMembership,
    membershipTier,
    membershipDuration,
    hasSubHistory,
    cumulativeMonths,
    handleSubscribeClick,
  };
}
