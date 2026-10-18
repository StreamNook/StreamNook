import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AnimatePresence, motion } from 'framer-motion';
import { MessageSquareText, Search, X } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { ChannelReward, ChannelRewardsSnapshot, RedeemOutcome, RewardBlock, UnlockedEmote } from '../types';

import { Logger } from '../utils/logger';
import { rewardStatusText } from '../utils/rewardStatus';
import { ChannelPointsIcon } from './ChannelPointsIcon';
import { Tooltip } from './ui/Tooltip';
import { useOutsidePress } from '../hooks/useOutsidePress';

/** Opening a reward re-reads the list when it is older than this. */
const REWARDS_STALE_MS = 30_000;
/** setTimeout's ceiling; longer cooldowns re-arm after a refresh. */
const MAX_TIMER_MS = 2_147_483_647;
/** Room the detail view and the emote pickers need, however short the list. */
const OVERLAY_MIN_HEIGHT = 340;
/** Shared with Twitch's own limit for a reward's typed text. */
const INPUT_MAX = 500;
/** The popover offers a search once the list is longer than this. */
const SEARCH_FROM = 8;

/** The dot's colour says what kind of wait it is: red for points, amber for
 *  time, the accent for a sub perk, grey for the streamer's own switches. */
const statusDot = (reason: RewardBlock | null): string => {
  switch (reason) {
    case 'not_enough_points':
      return 'bg-red-400';
    case 'on_cooldown':
    case 'only_while_live':
    case 'chat_reconnecting':
      return 'bg-yellow-400';
    case 'subscribers_only':
      return 'bg-accent-neon';
    default:
      return 'bg-textMuted';
  }
};

/** A status as a dot and words, never a chip. */
const StatusLine: React.FC<{ text: string; reason: RewardBlock | null; className?: string; title?: string }> = ({
  text,
  reason,
  className = '',
  title,
}) => (
  <span className={`flex items-start gap-1.5 min-w-0 text-xs leading-snug text-textSecondary ${className}`} role="status" title={title}>
    <span className={`w-1.5 h-1.5 mt-[5px] rounded-full flex-shrink-0 ${statusDot(reason)}`} aria-hidden="true" />
    <span className="line-clamp-2 break-words">{text}</span>
  </span>
);

/** A reward's picture standing free on a soft glow of its own colour, no tile
 *  around it. No picture, or one that fails to load, shows the points glyph. */
const RewardArt: React.FC<{ reward: ChannelReward; size: number; dim?: boolean }> = ({ reward, size, dim = false }) => {
  const [brokenUrl, setBrokenUrl] = useState<string | null>(null);
  const url = reward.image_url && reward.image_url !== brokenUrl ? reward.image_url : null;
  const color = reward.background_color || '#9147FF';
  return (
    <span
      className="relative flex flex-shrink-0 items-center justify-center"
      style={{ width: size + 10, height: size + 10 }}
      aria-hidden="true"
    >
      <span
        className={`absolute inset-0 rounded-full ${dim ? 'opacity-30' : 'opacity-90'}`}
        style={{ background: `radial-gradient(closest-side, color-mix(in srgb, ${color} 60%, transparent), transparent)` }}
      />
      {url ? (
        <img
          src={url}
          alt=""
          className={`relative rounded-[5px] object-contain drop-shadow-[0_1px_2px_rgba(0,0,0,0.45)] ${dim ? 'opacity-45 grayscale' : ''}`}
          style={{ width: size, height: size }}
          onError={() => setBrokenUrl(url)}
        />
      ) : (
        <ChannelPointsIcon size={Math.round(size * 0.8)} className={`relative text-white ${dim ? 'opacity-45' : ''}`} />
      )}
    </span>
  );
};

interface ChannelPointsMenuProps {
  channelLogin: string;  // Username for fetching rewards
  channelId: string;     // Numeric ID for redemption
  currentBalance: number | null;
  customPointsName?: string | null;
  customPointsIconUrl?: string | null;
  onClose: () => void;
  // Awaited when the user refreshes by hand, so the balance can show a pending
  // state for exactly as long as the fetch actually takes.
  onBalanceUpdate: () => void | Promise<void>;
  onEmotesChange?: () => void; // Callback to refresh emotes after unlocking
  /** `popover` (desktop): a panel over the composer. `pane` (desktop): the
   *  same body inside the channel wallet, which owns the glass, the motion,
   *  the height and the outside click. `sheet` (phone): the content of a
   *  bottom sheet, rewards as a grid of tiles. Same logic. */
  variant?: 'popover' | 'pane' | 'sheet';
  /** The button that toggles the popover: a press on it is its own toggle, not
   *  an outside click. */
  ignoreRef?: React.RefObject<HTMLElement | null>;
}

const ChannelPointsMenu: React.FC<ChannelPointsMenuProps> = ({
  channelLogin,
  channelId,
  currentBalance,
  customPointsName,
  customPointsIconUrl,
  onClose,
  onBalanceUpdate,
  onEmotesChange,
  variant = 'popover',
  ignoreRef,
}) => {
  const isSheet = variant === 'sheet';
  const isPane = variant === 'pane';
  const [rewards, setRewards] = useState<ChannelReward[]>([]);
  // The balance the backend judged the list against.
  const [snapshotBalance, setSnapshotBalance] = useState<number | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [redeemingId, setRedeemingId] = useState<string | null>(null);
  // Clock for the cooldown countdown text; ticks only while one is on screen.
  const [nowMs, setNowMs] = useState(() => Date.now());
  const fetchedAtRef = useRef(0);
  const fetchSeqRef = useRef(0);
  // The balance already refreshes on a timer and on earn/spend events. This is
  // the escape hatch for when it still looks stale.
  const [isRefreshingBalance, setIsRefreshingBalance] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  // Narrows a long list in the popover; the phone sheet scrolls instead.
  const [query, setQuery] = useState('');
  // The list's height when a search began, held while it runs: the popover
  // grows up from the composer, so a shrinking list would pull the search
  // field out from under the caret.
  const [searchHeight, setSearchHeight] = useState<number | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const changeQuery = (next: string) => {
    if (next === '') setSearchHeight(null);
    else if (query === '') setSearchHeight(listRef.current?.offsetHeight ?? null);
    setQuery(next);
  };
  // Motion Off opens and closes the popover in place. Reduced is MotionScope's
  // job: it keeps the fade and drops the movement.
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';

  const handleRefreshBalance = async () => {
    if (isRefreshingBalance) return;
    setIsRefreshingBalance(true);
    try {
      await onBalanceUpdate();
    } finally {
      setIsRefreshingBalance(false);
    }
  };

  // The reward opened in the detail view, and the text typed for it.
  const [detailReward, setDetailReward] = useState<ChannelReward | null>(null);
  const [inputText, setInputText] = useState('');

  // Emote reveal popup state
  const [showEmoteReveal, setShowEmoteReveal] = useState(false);
  const [revealedEmote, setRevealedEmote] = useState<UnlockedEmote | null>(null);

  // Modify emote picker modal state
  const [showModifyEmoteModal, setShowModifyEmoteModal] = useState(false);
  const [modifyEmoteReward, setModifyEmoteReward] = useState<ChannelReward | null>(null);
  const [modifiableEmotes, setModifiableEmotes] = useState<Array<{
    id: string;
    token: string;
    emote_type?: string;
    modifications: Array<{ id: string; modifier_id: string; token: string }>;
  }>>([]);
  const [isLoadingEmotes, setIsLoadingEmotes] = useState(false);
  const [selectedModifyEmote, setSelectedModifyEmote] = useState<string | null>(null);
  const [modifyStep, setModifyStep] = useState<'emote' | 'modifier'>('emote');
  const [selectedModifier, setSelectedModifier] = useState<{ id: string; modifier_id: string; token: string } | null>(null);

  // Choose emote modal state (single-step emote selection)
  const [showChooseEmoteModal, setShowChooseEmoteModal] = useState(false);
  const [chooseEmoteReward, setChooseEmoteReward] = useState<ChannelReward | null>(null);
  const [unlockableEmotes, setUnlockableEmotes] = useState<Array<{ id: string; token: string }>>([]);
  const [isLoadingUnlockable, setIsLoadingUnlockable] = useState(false);
  const [selectedUnlockEmote, setSelectedUnlockEmote] = useState<{ id: string; token: string } | null>(null);

  // Modifier display names and descriptions
  const MODIFIER_INFO: Record<string, { name: string; description: string }> = {
    'MOD_BW': { name: 'Black & White', description: 'Grayscale version' },
    'MOD_HF': { name: 'Flipped', description: 'Horizontally mirrored' },
    'MOD_SG': { name: 'Sunglasses', description: 'Wearing cool shades' },
    'MOD_SQ': { name: 'Squashed', description: 'Vertically squished' },
    'MOD_TK': { name: 'Thinking', description: 'With thinking hand' },
    'MOD_CV': { name: 'Cursed', description: 'Spooky distortion' },
  };

  // Read the reward list and the backend's per-reward verdicts. Only the
  // newest call applies; a failed background refresh keeps the list shown.
  const refreshRewards = useCallback((initial = false): Promise<void> => {
    const seq = ++fetchSeqRef.current;
    return invoke<ChannelRewardsSnapshot>('get_channel_rewards', {
      channelId: channelLogin  // Backend param is named channelId but expects login
    }).then(
      (snapshot) => {
        if (seq !== fetchSeqRef.current) return;
        fetchedAtRef.current = snapshot.fetched_at_ms;
        setRewards(snapshot.rewards);
        setSnapshotBalance(snapshot.balance);
        setError(null);
        setIsLoading(false);
      },
      (err: unknown) => {
        if (seq !== fetchSeqRef.current) return;
        Logger.error('[ChannelPointsMenu] Failed to fetch rewards:', err);
        // A list already on screen (Rust's last one) stays rather than an error.
        if (initial && fetchedAtRef.current === 0) {
          setError(typeof err === 'string' ? err : 'Failed to load rewards');
          setIsLoading(false);
        }
      },
    );
  }, [channelLogin]);

  // Fetch rewards when menu opens. Rust's last judged list for the channel
  // paints first, so a reopened menu never waits on Twitch; the fresh read
  // replaces it when it lands.
  useEffect(() => {
    const seqRef = fetchSeqRef;
    const seq = seqRef.current;
    void invoke<ChannelRewardsSnapshot | null>('get_cached_channel_rewards', { channelLogin })
      .then((cached) => {
        // Never over the fresh read, and never after an unmount or a channel switch.
        if (!cached || seqRef.current !== seq + 1 || fetchedAtRef.current >= cached.fetched_at_ms) return;
        fetchedAtRef.current = cached.fetched_at_ms;
        setRewards(cached.rewards);
        setSnapshotBalance(cached.balance);
        setIsLoading(false);
      })
      .catch(() => {});
    void refreshRewards(true);
    // Unmounting (or a channel switch) voids any call still in flight.
    return () => { seqRef.current++; };
  }, [refreshRewards, channelLogin]);

  // Rust re-judges the list when chat enters or leaves sub-only mode, the
  // viewer's sub badge changes, or the stream goes live or offline.
  useEffect(() => {
    const login = channelLogin.toLowerCase();
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<ChannelRewardsSnapshot>('channel-rewards', (event) => {
      if (event.payload.login !== login) return;
      setRewards(event.payload.rewards);
      setSnapshotBalance(event.payload.balance);
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [channelLogin]);

  const retryRewards = () => {
    setIsLoading(true);
    setError(null);
    void refreshRewards(true);
  };

  // Re-read a list older than REWARDS_STALE_MS; a no-op otherwise.
  const refreshIfStale = useCallback(async () => {
    if (Date.now() - fetchedAtRef.current <= REWARDS_STALE_MS) return;
    await refreshRewards();
  }, [refreshRewards]);

  // Points were earned or spent since the verdicts were made: have them
  // judged again against the new balance.
  useEffect(() => {
    if (currentBalance === null || snapshotBalance === null) return;
    if (currentBalance === snapshotBalance) return;
    void refreshRewards();
  }, [currentBalance, snapshotBalance, refreshRewards]);

  // One timer to the soonest cooldown end, then a fresh verdict.
  const nextCooldownEnd = useMemo(
    () => rewards.reduce<number | null>((soonest, r) => {
      const until = r.availability.cooldown_until_ms;
      return until !== null && (soonest === null || until < soonest) ? until : soonest;
    }, null),
    [rewards],
  );
  useEffect(() => {
    if (nextCooldownEnd === null) return;
    // A beat past the end, so the backend judges it after the cooldown.
    const delay = Math.min(Math.max(0, nextCooldownEnd - Date.now()) + 250, MAX_TIMER_MS);
    const timer = window.setTimeout(() => { void refreshRewards(); }, delay);
    return () => window.clearTimeout(timer);
  }, [nextCooldownEnd, refreshRewards]);

  // Presentational countdown tick, only while a cooldown text is visible.
  const showsCooldown = rewards.some((r) => r.availability.reason === 'on_cooldown');
  useEffect(() => {
    if (!showsCooldown) return;
    const tick = () => setNowMs(Date.now());
    const first = window.setTimeout(tick, 0);
    const interval = window.setInterval(tick, 1000);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(interval);
    };
  }, [showsCooldown]);

  // A view holds the reward it opened with; read its current verdict from the
  // latest list. A reward that dropped out of the list cannot be redeemed.
  const liveOf = (reward: ChannelReward): ChannelReward => {
    const live = rewards.find((r) => r.id === reward.id);
    if (live) return live;
    return {
      ...reward,
      availability: {
        ...reward.availability,
        redeemable: false,
        reason: 'disabled',
        cooldown_until_ms: null,
        label: 'Gone',
        detail: 'This reward is no longer offered',
      },
    };
  };

  const closeDetail = useCallback(() => {
    setDetailReward(null);
    setInputText('');
  }, []);

  // A press outside the popover closes it, as the commands menu beside it
  // does. A sheet closes on its own backdrop, a pane with its wallet.
  const markInside = useOutsidePress(!isSheet && !isPane, [menuRef, ignoreRef], onClose);

  // Escape backs out of the open view first, then clears a search, then
  // closes the menu.
  const detailOpen = detailReward !== null;
  const searching = query !== '';
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (detailOpen) {
        closeDetail();
        return;
      }
      if (searching) {
        setQuery('');
        setSearchHeight(null);
        return;
      }
      onClose();
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose, closeDetail, detailOpen, searching]);

  // A click on any row opens its detail, available or not, so the full name,
  // the description and the reason it is blocked are always one click away.
  const openDetail = (reward: ChannelReward) => {
    setDetailReward(reward);
    setInputText('');
    // The verdict may have gone stale while the menu sat open (a reward
    // paused, a cooldown started elsewhere). The view follows the fresh list
    // as soon as it lands.
    void refreshIfStale();
  };

  const openModifyPicker = async (reward: ChannelReward) => {
    setModifyEmoteReward(reward);
    setSelectedModifyEmote(null);
    setShowModifyEmoteModal(true);
    setIsLoadingEmotes(true);
    try {
      const emotes = await invoke<Array<{
        id: string;
        token: string;
        emote_type?: string;
        modifications: Array<{ id: string; modifier_id: string; token: string }>;
      }>>('get_modifiable_emotes', {
        channelId: channelLogin, // Use login for ChannelPointsContext query
      });
      setModifiableEmotes(emotes);
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Failed to fetch modifiable emotes:', err);
      useAppStore.getState().addToast('Failed to load emotes', 'error');
    } finally {
      setIsLoadingEmotes(false);
    }
  };

  const openChoosePicker = async (reward: ChannelReward) => {
    setChooseEmoteReward(reward);
    setSelectedUnlockEmote(null);
    setShowChooseEmoteModal(true);
    setIsLoadingUnlockable(true);
    try {
      const emotes = await invoke<Array<{
        id: string;
        token: string;
        emote_type?: string;
        modifications: Array<{ id: string; modifier_id: string; token: string }>;
      }>>('get_modifiable_emotes', {
        channelId: channelLogin,
      });
      // Just the base emotes, without their modified versions.
      setUnlockableEmotes(emotes.map(e => ({ id: e.id, token: e.token })));
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Failed to fetch unlockable emotes:', err);
      useAppStore.getState().addToast('Failed to load emotes', 'error');
    } finally {
      setIsLoadingUnlockable(false);
    }
  };

  // The detail view's main button: redeem, send the typed text, or move on
  // to the emote picker the reward needs.
  const handleDetailAction = async () => {
    if (!detailReward || redeemingId) return;
    const reward = liveOf(detailReward);
    if (!reward.availability.redeemable) return;
    const kind = reward.view.kind;

    if (kind === 'modify_emote') {
      closeDetail();
      void openModifyPicker(reward);
      return;
    }
    if (kind === 'choose_emote') {
      closeDetail();
      void openChoosePicker(reward);
      return;
    }

    const text = inputText.trim();
    if (reward.view.takes_text && !text) return;

    setRedeemingId(reward.id);
    try {
      // Rust routes by the reward's type and keeps the retry-safe transaction id.
      const result = await invoke<RedeemOutcome>('redeem_reward', {
        channelLogin: channelLogin,
        channelId: channelId,
        rewardId: reward.id,
        text: text,
      });
      if (result.success) {
        closeDetail();
        if (kind === 'random_emote') {
          if (result.unlocked_emote) {
            setRevealedEmote(result.unlocked_emote);
            setShowEmoteReveal(true);
          } else {
            useAppStore.getState().addToast(result.message, 'success');
          }
          if (onEmotesChange) onEmotesChange();
        } else {
          useAppStore.getState().addToast(result.message, 'success');
        }
        onBalanceUpdate();
        void refreshRewards(); // A redeem can start a cooldown or use up stock
      } else {
        useAppStore.getState().addToast(result.message, 'error');
      }
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Redemption error:', err);
      useAppStore.getState().addToast(typeof err === 'string' ? err : 'Redemption failed', 'error');
    } finally {
      setRedeemingId(null);
    }
  };

  // Handle confirmation of modified emote unlock
  const handleModifyEmoteConfirm = async () => {
    if (!modifyEmoteReward || !selectedModifyEmote || !selectedModifier) {
      useAppStore.getState().addToast('Please select an emote and modifier', 'error');
      return;
    }
    if (!liveOf(modifyEmoteReward).availability.redeemable) return;

    // Use the full modified emote ID from the API (e.g., "1022569_BW")
    const finalEmoteId = selectedModifier.id;

    setShowModifyEmoteModal(false);
    setRedeemingId(modifyEmoteReward.id);

    try {
      const result = await invoke<RedeemOutcome>('redeem_reward', {
        channelLogin: channelLogin,
        channelId: channelId,
        rewardId: modifyEmoteReward.id,
        emoteId: finalEmoteId,
        emoteName: selectedModifier.token,
      });

      if (result.success) {
        // Show emote reveal popup with the token (actual emote name like "hamzDead_BW")
        // Use the token from selectedModifier since backend doesn't have it
        const emoteToReveal = {
          id: selectedModifier.id,
          name: selectedModifier.token, // Use the actual emote name, not the numeric ID
          image_url: `https://static-cdn.jtvnw.net/emoticons/v2/${selectedModifier.id}/default/dark/2.0`,
        };
        setRevealedEmote(emoteToReveal);
        setShowEmoteReveal(true);
        onBalanceUpdate();
        void refreshRewards();
        // Trigger emote refresh
        if (onEmotesChange) {
          onEmotesChange();
        }
      } else {
        useAppStore.getState().addToast(result.message, 'error');
      }
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Modify emote error:', err);
      useAppStore.getState().addToast(typeof err === 'string' ? err : 'Failed to modify emote', 'error');
    } finally {
      setRedeemingId(null);
      setModifyEmoteReward(null);
      setSelectedModifyEmote(null);
      setSelectedModifier(null);
      setModifyStep('emote');
    }
  };

  // Handle confirmation of chosen emote unlock
  const handleChooseEmoteConfirm = async () => {
    if (!chooseEmoteReward || !selectedUnlockEmote) {
      useAppStore.getState().addToast('Please select an emote', 'error');
      return;
    }
    if (!liveOf(chooseEmoteReward).availability.redeemable) return;

    setShowChooseEmoteModal(false);
    setRedeemingId(chooseEmoteReward.id);

    try {
      const result = await invoke<RedeemOutcome>('redeem_reward', {
        channelLogin: channelLogin,
        channelId: channelId,
        rewardId: chooseEmoteReward.id,
        emoteId: selectedUnlockEmote.id,
        emoteName: selectedUnlockEmote.token,
      });

      if (result.success) {
        // Show emote reveal popup
        const emoteToReveal = {
          id: selectedUnlockEmote.id,
          name: selectedUnlockEmote.token,
          image_url: `https://static-cdn.jtvnw.net/emoticons/v2/${selectedUnlockEmote.id}/default/dark/2.0`,
        };
        setRevealedEmote(emoteToReveal);
        setShowEmoteReveal(true);
        onBalanceUpdate();
        void refreshRewards();
        // Trigger emote refresh
        if (onEmotesChange) {
          onEmotesChange();
        }
      } else {
        useAppStore.getState().addToast(result.message, 'error');
      }
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Choose emote error:', err);
      useAppStore.getState().addToast(typeof err === 'string' ? err : 'Failed to unlock emote', 'error');
    } finally {
      setRedeemingId(null);
      setChooseEmoteReward(null);
      setSelectedUnlockEmote(null);
    }
  };

  // Current verdicts for whichever view is open.
  const detail = detailReward ? liveOf(detailReward) : null;
  const detailKind = detail ? detail.view.kind : null;
  const detailTakesText = !!detail?.view.takes_text;
  const detailStatus = detail ? rewardStatusText(detail.availability, nowMs, 'detail') : null;
  const detailAbout = detail ? (detail.prompt?.trim() || detail.description?.trim() || null) : null;
  const detailBusy = detail !== null && redeemingId === detail.id;
  const detailCanAct = detail !== null
    && detail.availability.redeemable
    && redeemingId === null
    && (!detailTakesText || inputText.trim().length > 0);
  // Rust's words for the reward's action and its busy state.
  const detailActionLabel = detail ? (detailBusy ? detail.view.busy : detail.view.action) : '';
  const modifyAvail = modifyEmoteReward ? liveOf(modifyEmoteReward).availability : null;
  const chooseAvail = chooseEmoteReward ? liveOf(chooseEmoteReward).availability : null;
  const modifyBlocked = modifyAvail ? rewardStatusText(modifyAvail, nowMs, 'detail') : null;
  const chooseBlocked = chooseAvail ? rewardStatusText(chooseAvail, nowMs, 'detail') : null;
  const overlayOpen = detail !== null || showModifyEmoteModal || showChooseEmoteModal || showEmoteReveal;

  const pointsGlyph = (size: number, className = '') => (customPointsIconUrl ? (
    <img src={customPointsIconUrl} alt="" style={{ width: size, height: size }} className={className} />
  ) : (
    <ChannelPointsIcon size={size} className={className} />
  ));

  const pointsName = customPointsName || 'Channel Points';
  const balance = currentBalance ?? snapshotBalance;
  const showSearch = !isSheet && rewards.length > SEARCH_FROM;
  const q = showSearch ? query.trim().toLowerCase() : '';
  const shownRewards = q
    ? rewards.filter((r) => `${r.title}\n${r.prompt ?? ''}\n${r.description ?? ''}`.toLowerCase().includes(q))
    : rewards;

  // The popover rises from the points button at the composer's left edge.
  const popoverMotion = isSheet || isPane
    ? {}
    : {
        initial: { opacity: 0, y: 6, scale: 0.98 },
        animate: { opacity: 1, y: 0, scale: 1 },
        exit: { opacity: 0, y: 6, scale: 0.98 },
        transition: motionOff ? { duration: 0 } : { duration: 0.16, ease: 'easeOut' as const },
      };

  const balanceButton = (
    <Tooltip content="Click to refresh" side="top">
      <button
        type="button"
        onClick={handleRefreshBalance}
        disabled={isRefreshingBalance}
        aria-label={`Refresh ${customPointsName || 'channel points'} balance`}
        className="flex items-center gap-1.5 rounded-md px-1.5 py-0.5 -mr-1.5 transition-colors hover:bg-surface-hover disabled:opacity-60 flex-shrink-0"
      >
        {customPointsIconUrl ? (
          <img
            src={customPointsIconUrl}
            alt=""
            className={`w-4 h-4 ${isRefreshingBalance ? 'animate-spin' : ''}`}
          />
        ) : (
          <ChannelPointsIcon
            size={16}
            className={`text-accent-neon ${isRefreshingBalance ? 'animate-spin' : ''}`}
          />
        )}
        <span className="text-base font-bold text-accent-neon">
          {balance?.toLocaleString() ?? '--'}
        </span>
      </button>
    </Tooltip>
  );

  const errorOrEmpty = error ? (
    <div className="px-4 py-6 text-center">
      <p className="text-textSecondary text-sm">{error}</p>
      <button
        onClick={retryRewards}
        className="mt-2 text-xs text-accent hover:underline"
      >
        Try again
      </button>
    </div>
  ) : (
    <div className="px-4 py-6 text-center">
      <p className="text-textSecondary text-sm">No rewards available</p>
      <p className="text-textMuted text-xs mt-1">
        This channel hasn't set up any rewards yet
      </p>
    </div>
  );

  return (
    <motion.div
      ref={menuRef}
      onPointerDownCapture={markInside}
      {...popoverMotion}
      className={
        isSheet
          ? 'relative flex flex-col'
          : isPane
            ? 'relative flex min-h-0 flex-1 flex-col'
          // glass-flyout: this can open over live video, and its base stays
          // readable at every Glassiness.
          : 'glass-flyout absolute bottom-full left-0 right-0 mb-2 flex flex-col max-h-[400px] overflow-hidden z-50 origin-bottom-left !p-0'
      }
      style={{ minHeight: overlayOpen ? OVERLAY_MIN_HEIGHT : undefined }}
    >
      {isSheet ? (
        <>
          {/* Header with balance */}
          <div className={`px-1 pb-3 ${overlayOpen ? 'invisible' : ''}`}>
            <div className="flex items-center justify-between gap-2">
              <span className="text-sm font-medium text-textSecondary truncate">
                {pointsName}
              </span>
              {balanceButton}
            </div>
          </div>

          {/* Content. The sheet scrolls. */}
          <div className={overlayOpen ? 'invisible' : ''}>
            {isLoading ? (
              <div className="flex items-center justify-center py-8">
                <div className="animate-spin w-6 h-6 border-2 border-accent border-t-transparent rounded-full" />
              </div>
            ) : error || rewards.length === 0 ? (
              errorOrEmpty
            ) : (
              // Tiles, Twitch's own layout on a phone: the art, the cost under it,
              // the name on two lines, and why it is blocked when it is.
              <div className="grid grid-cols-3 gap-2">
                {rewards.map((reward) => {
                  const available = reward.availability.redeemable;
                  const status = rewardStatusText(reward.availability, nowMs, 'label');
                  const canAfford = reward.availability.affordable;
                  return (
                    <button
                      key={reward.id}
                      type="button"
                      onClick={() => openDetail(reward)}
                      aria-label={status ? `${reward.title}, ${status}` : reward.title}
                      className={`flex min-w-0 flex-col gap-1.5 rounded-xl p-1.5 text-left transition-colors active:bg-surface-hover ${
                        redeemingId === reward.id ? 'animate-pulse bg-accent/10' : ''
                      }`}
                    >
                      <div
                        className={`relative flex aspect-square w-full items-center justify-center overflow-hidden rounded-lg ${available ? '' : 'opacity-50 grayscale'}`}
                        style={{ backgroundColor: reward.background_color || '#9147FF' }}
                      >
                        {reward.image_url ? (
                          <img
                            src={reward.image_url}
                            alt=""
                            className="h-3/5 w-3/5 object-contain"
                            onError={(e) => {
                              e.currentTarget.style.display = 'none';
                            }}
                          />
                        ) : (
                          <ChannelPointsIcon size={26} className="text-white/85" />
                        )}
                      </div>
                      <span
                        className={`flex items-center gap-1 text-[12px] font-semibold tabular-nums ${
                          canAfford ? (available ? 'text-accent' : 'text-textSecondary') : 'text-red-400'
                        }`}
                      >
                        {pointsGlyph(12)}
                        {reward.view.cost_label}
                      </span>
                      <span
                        className={`line-clamp-2 break-words text-[12px] leading-snug ${available ? 'text-textPrimary' : 'text-textSecondary'}`}
                      >
                        {reward.title}
                      </span>
                      {status && <StatusLine text={status} reason={reward.availability.reason} />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </>
      ) : (
        <>
          {/* The balance leads; a search joins it once the list is long. */}
          <div className={`flex items-center gap-2 border-b border-white/[0.06] py-2 pl-2 pr-2.5 ${overlayOpen ? 'invisible' : ''}`}>
            <Tooltip content="Click to refresh" side="top">
              <button
                type="button"
                onClick={handleRefreshBalance}
                disabled={isRefreshingBalance}
                aria-label={`${pointsName}: ${balance?.toLocaleString() ?? 'unknown'}. Refresh`}
                className="flex min-w-0 items-center gap-2 rounded-lg px-1.5 py-1 transition-colors hover:bg-white/[0.05] disabled:opacity-70"
              >
                {customPointsIconUrl ? (
                  <img
                    src={customPointsIconUrl}
                    alt=""
                    className={`h-[18px] w-[18px] flex-shrink-0 ${isRefreshingBalance ? 'animate-spin' : ''}`}
                  />
                ) : (
                  <ChannelPointsIcon
                    size={18}
                    className={`flex-shrink-0 text-accent-neon ${isRefreshingBalance ? 'animate-spin' : ''}`}
                  />
                )}
                <span className="text-[16px] font-bold leading-none tabular-nums text-textPrimary">
                  {balance?.toLocaleString() ?? '--'}
                </span>
                <span className="truncate text-[12px] leading-none text-textMuted">{pointsName}</span>
              </button>
            </Tooltip>
            {showSearch && (
              <label className="glass-input ml-auto flex h-7 w-[38%] min-w-[96px] max-w-[140px] flex-shrink-0 items-center gap-1.5 px-2 focus-within:border-white/20">
                <Search size={12} className="flex-shrink-0 text-textMuted" aria-hidden="true" />
                <input
                  value={query}
                  onChange={(e) => changeQuery(e.target.value)}
                  placeholder="Find a reward"
                  spellCheck={false}
                  aria-label="Find a reward"
                  className="min-w-0 flex-1 bg-transparent text-[12px] text-textPrimary outline-none placeholder:text-textMuted"
                />
                {query && (
                  <button
                    type="button"
                    onClick={() => changeQuery('')}
                    aria-label="Clear the search"
                    className="flex-shrink-0 text-textMuted transition-colors hover:text-textPrimary"
                  >
                    <X size={12} />
                  </button>
                )}
              </label>
            )}
          </div>

          <div
            ref={listRef}
            className={`custom-scrollbar ${isPane ? 'min-h-0 flex-1' : 'max-h-[320px]'} overflow-y-auto py-1.5 ${overlayOpen ? 'invisible' : ''}`}
            style={{ minHeight: q ? (searchHeight ?? undefined) : undefined }}
          >
            {isLoading ? (
              // The shape of the list, so it settles in place when it lands.
              <div className="animate-pulse" aria-busy="true" aria-label="Loading rewards">
                {[0.62, 0.48, 0.7, 0.4, 0.56, 0.44].map((w) => (
                  <div key={w} className="mx-1.5 flex items-center gap-2.5 px-2 py-2">
                    <span className="h-[38px] w-[38px] flex-shrink-0 rounded-full bg-white/[0.05]" />
                    <span className="flex min-w-0 flex-1 flex-col gap-1.5">
                      <span className="block h-2.5 rounded-full bg-white/[0.07]" style={{ width: `${w * 100}%` }} />
                      <span className="block h-2 w-1/3 rounded-full bg-white/[0.04]" />
                    </span>
                    <span className="h-2.5 w-9 flex-shrink-0 rounded-full bg-white/[0.06]" />
                  </div>
                ))}
              </div>
            ) : error || rewards.length === 0 ? (
              errorOrEmpty
            ) : shownRewards.length === 0 ? (
              <p className="px-4 py-6 text-center text-[12px] text-textMuted">No rewards match “{query.trim()}”</p>
            ) : (
              // Rust drops a reward that turned off or sold out while the menu
              // is open; it folds away (and one coming back unfolds) instead of
              // the rows below jumping.
              <AnimatePresence initial={false}>
              {shownRewards.map((reward) => {
                const available = reward.availability.redeemable;
                const status = rewardStatusText(reward.availability, nowMs, 'label');
                const about = reward.prompt?.trim() || reward.description?.trim() || null;
                const takesText = reward.view.takes_text;
                const canAfford = reward.availability.affordable;
                // How close the balance is to a reward it cannot cover yet,
                // only while points are what stands in the way.
                const progress = !canAfford && reward.availability.reason === 'not_enough_points'
                  && balance !== null && reward.cost > 0
                  ? Math.max(0.04, Math.min(1, balance / reward.cost))
                  : null;

                return (
                  <motion.div
                    key={reward.id}
                    className="overflow-hidden px-1.5"
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    exit={{ opacity: 0, height: 0 }}
                    transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
                  >
                    <button
                      type="button"
                      onClick={() => openDetail(reward)}
                      aria-label={status ? `${reward.title}, ${status}` : reward.title}
                      className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-white/[0.05] focus-visible:bg-white/[0.06] focus-visible:outline-none ${
                        redeemingId === reward.id ? 'animate-pulse bg-white/[0.05]' : ''
                      }`}
                    >
                      <RewardArt reward={reward} size={28} dim={!available} />

                      {/* Full name on up to two lines, then why it is blocked or what it asks */}
                      <span className="min-w-0 flex-1">
                        <span
                          className={`text-[13px] font-semibold leading-snug line-clamp-2 break-words ${available ? 'text-textPrimary' : 'text-textSecondary'}`}
                          title={reward.title}
                        >
                          {reward.title}
                        </span>
                        {status ? (
                          <StatusLine
                            text={status}
                            reason={reward.availability.reason}
                            className="mt-0.5 !text-[11.5px]"
                            title={reward.availability.detail ?? undefined}
                          />
                        ) : about || takesText ? (
                          <span className="mt-0.5 flex min-w-0 items-center gap-1 text-[11.5px] leading-snug text-textMuted">
                            {takesText && <MessageSquareText size={11} className="flex-shrink-0" aria-label="Asks for a message" />}
                            <span className="truncate">{about ?? 'Asks for a message'}</span>
                          </span>
                        ) : null}
                      </span>

                      {/* Cost, always visible; under it, how far off a reward out of reach is */}
                      <span className="flex flex-shrink-0 flex-col items-end gap-1">
                        <span
                          className={`flex items-center gap-1 text-[12.5px] font-semibold tabular-nums ${available ? 'text-textPrimary' : 'text-textMuted'}`}
                        >
                          {pointsGlyph(12, available ? 'text-accent-neon' : '')}
                          {reward.view.cost_label}
                        </span>
                        {progress !== null && (
                          <span className="block h-0.5 w-9 overflow-hidden rounded-full bg-white/[0.08]" aria-hidden="true">
                            <span className="block h-full rounded-full bg-red-400/70" style={{ width: `${progress * 100}%` }} />
                          </span>
                        )}
                      </span>
                    </button>
                  </motion.div>
                );
              })}
              </AnimatePresence>
            )}
          </div>
        </>
      )}

      {/* Reward detail: the full name, what it does, why it is blocked, and
          the text box for rewards that ask for one */}
      {detail && (
        <div
          className="animate-scale-in absolute inset-0 flex flex-col overflow-hidden z-50"
          role="dialog"
          aria-label={detail.title}
        >
          <div className={`flex items-center justify-between gap-2 px-2 py-2 border-b ${isSheet ? 'border-borderSubtle' : 'border-white/[0.06]'}`}>
            <button
              type="button"
              onClick={closeDetail}
              className="flex items-center gap-1 px-1.5 py-1 text-xs font-medium text-textSecondary hover:text-textPrimary hover:bg-glass rounded-md transition-colors"
              aria-label="Back to rewards"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
              </svg>
              Rewards
            </button>
            {isSheet ? (
              <div className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full text-sm font-bold tabular-nums ${detail.availability.affordable ? 'bg-accent/20 text-accent' : 'bg-red-500/15 text-red-400'}`}>
                {pointsGlyph(14)}
                {detail.cost.toLocaleString()}
              </div>
            ) : (
              <div className={`flex items-center gap-1.5 pr-1.5 text-sm font-bold tabular-nums ${detail.availability.affordable ? 'text-textPrimary' : 'text-textMuted'}`}>
                {pointsGlyph(14, detail.availability.affordable ? 'text-accent-neon' : '')}
                {detail.cost.toLocaleString()}
              </div>
            )}
          </div>

          <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-4 pt-3 pb-2 flex flex-col gap-2.5">
            <div className="flex items-center gap-3">
              {isSheet ? (
                <div
                  className="w-10 h-10 rounded-lg flex items-center justify-center flex-shrink-0 overflow-hidden"
                  style={{ backgroundColor: detail.background_color || '#9147FF' }}
                >
                  {detail.image_url ? (
                    <img src={detail.image_url} alt="" className="w-full h-full object-cover" />
                  ) : (
                    <ChannelPointsIcon size={20} className="text-white/80" />
                  )}
                </div>
              ) : (
                <RewardArt reward={detail} size={34} dim={!detail.availability.redeemable} />
              )}
              <h3 className={`text-sm font-semibold leading-snug break-words min-w-0 ${detailKind === 'highlight' ? 'text-yellow-400' : 'text-textPrimary'}`}>
                {detail.title}
              </h3>
            </div>
            {detailAbout && (
              <p className="text-xs text-textSecondary leading-relaxed whitespace-pre-wrap break-words">
                {detailAbout}
              </p>
            )}
            {detailStatus && (
              <span
                className="flex items-start gap-1.5 text-xs text-textPrimary leading-snug"
                role="status"
              >
                <span className={`w-1.5 h-1.5 mt-[5px] rounded-full flex-shrink-0 ${statusDot(detail.availability.reason)}`} aria-hidden="true" />
                <span className="break-words">{detailStatus}</span>
              </span>
            )}
            {detailTakesText && detail.availability.redeemable && (
              <textarea
                value={inputText}
                onChange={(e) => setInputText(e.target.value)}
                placeholder={detailKind === 'input' ? 'Type your answer...' : 'Type your message...'}
                maxLength={INPUT_MAX}
                rows={3}
                autoFocus
                className={`w-full min-h-[72px] glass-input px-3 py-2 ${isSheet ? 'text-[15px]' : 'text-sm'} text-textPrimary placeholder-textMuted resize-none focus:outline-none`}
                onKeyDown={(e) => {
                  // Enter that confirms an IME candidate is not a send: it would
                  // post half-typed text and spend the points.
                  if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    void handleDetailAction();
                  }
                }}
              />
            )}
          </div>

          <div className={`flex items-center justify-between gap-2 px-4 py-2.5 border-t ${isSheet ? 'border-borderSubtle' : 'border-white/[0.06]'}`}>
            {detailStatus ? (
              <span aria-hidden="true" />
            ) : detailTakesText ? (
              <span className="text-xs text-textMuted tabular-nums">{inputText.length}/{INPUT_MAX}</span>
            ) : (
              <span className="text-xs text-textMuted">Can't be undone</span>
            )}
            <button
              type="button"
              onClick={() => { void handleDetailAction(); }}
              disabled={!detailCanAct}
              className={`
                flex-shrink-0 ${isSheet ? 'px-5 py-2.5 text-sm' : 'px-4 py-1.5 text-xs'} font-semibold glass-button transition-all whitespace-nowrap
                ${detailCanAct
                  ? (detailKind === 'highlight' ? 'text-yellow-400 hover:text-yellow-300' : 'text-accent hover:text-accent-neon active:scale-[0.98]')
                  : 'opacity-50 cursor-not-allowed'}
              `}
            >
              {detailActionLabel}
            </button>
          </div>
        </div>
      )}

      {/* Emote Reveal Popup */}
      {showEmoteReveal && revealedEmote && (
        <div 
          className={`animate-scale-in absolute inset-0 flex flex-col items-center justify-center overflow-hidden z-50`}
          onClick={() => {
            setShowEmoteReveal(false);
            setRevealedEmote(null);
          }}
        >
          {/* Sparkle background effect */}
          <div className="absolute inset-0 overflow-hidden pointer-events-none">
            <div className="absolute top-1/4 left-1/4 w-2 h-2 bg-yellow-400 rounded-full animate-ping" style={{ animationDelay: '0s' }} />
            <div className="absolute top-1/3 right-1/4 w-1.5 h-1.5 bg-purple-400 rounded-full animate-ping" style={{ animationDelay: '0.2s' }} />
            <div className="absolute bottom-1/3 left-1/3 w-2 h-2 bg-cyan-400 rounded-full animate-ping" style={{ animationDelay: '0.4s' }} />
            <div className="absolute top-1/2 right-1/3 w-1 h-1 bg-pink-400 rounded-full animate-ping" style={{ animationDelay: '0.6s' }} />
          </div>

          {/* Title */}
          <div className="text-yellow-400 text-lg font-bold mb-4 animate-pulse">
            🎉 You unlocked an emote! 🎉
          </div>

          {/* Emote image */}
          <div className="relative mb-4">
            <div 
              className="absolute inset-0 bg-gradient-to-r from-purple-500 via-pink-500 to-yellow-500 rounded-xl animate-spin-slow opacity-50 blur-xl"
              style={{ animation: 'spin 3s linear infinite' }}
            />
            <img
              src={revealedEmote.image_url}
              alt={revealedEmote.name}
              className="relative w-24 h-24 object-contain rounded-lg bg-glass/50 p-2"
              style={{ imageRendering: 'pixelated' }}
            />
          </div>

          {/* Emote name */}
          <div className="text-2xl font-bold text-textPrimary mb-2">
            {revealedEmote.name}
          </div>

          {/* Tap to dismiss */}
          <div className="text-xs text-textMuted mt-4">
            Tap to dismiss
          </div>
        </div>
      )}

      {/* Modify Emote Picker Modal */}
      {showModifyEmoteModal && modifyEmoteReward && (
        <div 
          className={`animate-scale-in absolute inset-0 z-50 flex flex-col overflow-hidden`}
        >
          {/* Header */}
          <div className="flex items-center justify-between p-4 border-b border-borderSubtle">
            <div className="flex items-center gap-3">
              <button
                onClick={() => {
                  if (modifyStep === 'modifier') {
                    // Go back to emote selection
                    setModifyStep('emote');
                    setSelectedModifier(null);
                  } else {
                    // Close modal
                    setShowModifyEmoteModal(false);
                    setModifyEmoteReward(null);
                    setSelectedModifyEmote(null);
                    setSelectedModifier(null);
                    setModifyStep('emote');
                  }
                }}
                className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded-lg transition-all"
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  {modifyStep === 'modifier' ? (
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                  ) : (
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  )}
                </svg>
              </button>
              <div>
                <h3 className="text-base font-semibold text-textPrimary">
                  {modifyStep === 'emote' ? 'Select an Emote' : 'Choose Modification'}
                </h3>
                <p className="text-xs text-textSecondary">{modifyEmoteReward.title}</p>
              </div>
            </div>
            <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-accent/20">
              {customPointsIconUrl ? (
                <img src={customPointsIconUrl} alt="" className="w-4 h-4" />
              ) : (
                <ChannelPointsIcon size={14} className="text-accent" />
              )}
              <span className="text-sm font-bold text-accent">
                {modifyEmoteReward.cost.toLocaleString()}
              </span>
            </div>
          </div>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-4 scrollbar-thin">
            {modifyStep === 'emote' ? (
              // Step 1: Emote Selection
              <>
                {isLoadingEmotes ? (
                  <div className="flex items-center justify-center h-32">
                    <div className="flex flex-col items-center gap-2">
                      <div className="w-8 h-8 border-2 border-accent border-t-transparent rounded-full animate-spin" />
                      <span className="text-sm text-textSecondary">Loading emotes...</span>
                    </div>
                  </div>
                ) : modifiableEmotes.length === 0 ? (
                  <div className="flex items-center justify-center h-32">
                    <p className="text-sm text-textSecondary">No emotes available to modify</p>
                  </div>
                ) : (
                  <div className="grid grid-cols-6 gap-1.5">
                    {modifiableEmotes.map((emote) => {
                      const baseId = emote.id.split('_')[0];
                      const imageUrl = `https://static-cdn.jtvnw.net/emoticons/v2/${baseId}/default/dark/2.0`;
                      const isSelected = selectedModifyEmote === emote.id;

                      return (
                        <button
                          key={emote.id}
                          onClick={() => setSelectedModifyEmote(emote.id)}
                          className={`
                            relative flex flex-col items-center gap-1 p-2 rounded-lg transition-all border border-transparent
                            ${isSelected
                              ? 'bg-surface-active border-border text-accent-neon shadow-inner'
                              : 'hover:bg-surface-hover'
                            }
                          `}
                        >
                          <img
                            src={imageUrl}
                            alt={emote.token}
                            className="w-8 h-8 object-contain"
                            onError={(e) => {
                              e.currentTarget.style.display = 'none';
                            }}
                          />
                          <span className="text-[10px] text-textSecondary truncate w-full text-center">
                            {emote.token}
                          </span>
                          {isSelected && (
                            <div className="absolute top-1 right-1 w-4 h-4 bg-accent rounded-full flex items-center justify-center">
                              <svg className="w-2.5 h-2.5 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={3} d="M5 13l4 4L19 7" />
                              </svg>
                            </div>
                          )}
                        </button>
                      );
                    })}
                  </div>
                )}
              </>
            ) : (
              // Step 2: Modifier Selection
              <div className="flex flex-col gap-4">
                {/* Modifier Grid - show modifications for selected emote */}
                <div className="grid grid-cols-2 gap-2">
                  {modifiableEmotes.find(e => e.id === selectedModifyEmote)?.modifications.map((modification) => {
                    const isSelected = selectedModifier?.id === modification.id;
                    const modInfo = MODIFIER_INFO[modification.modifier_id] || { name: modification.token, description: 'Modified emote' };
                    const shortCode = modification.id.split('_').pop() || '';
                    return (
                      <button
                        key={modification.id}
                        onClick={() => setSelectedModifier(modification)}
                        className={`
                          flex items-center gap-3 p-3 rounded-lg transition-all text-left border border-transparent
                          ${isSelected
                            ? 'bg-surface-active border-border text-accent-neon shadow-inner'
                            : 'hover:bg-surface-hover'
                          }
                        `}
                      >
                        <img
                          src={`https://static-cdn.jtvnw.net/emoticons/v2/${modification.id}/default/dark/2.0`}
                          alt={modification.token}
                          className="w-10 h-10 object-contain rounded-lg"
                          onError={(e) => {
                            // Fallback to showing short code if image fails
                            e.currentTarget.style.display = 'none';
                          }}
                        />
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium text-textPrimary">{modInfo.name}</p>
                          <p className="text-[10px] text-textSecondary truncate">{shortCode} - {modInfo.description}</p>
                        </div>
                        {isSelected && (
                          <svg className="w-5 h-5 text-accent flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                          </svg>
                        )}
                      </button>
                    );
                  })}
                </div>
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="flex items-center justify-between p-4 border-t border-borderSubtle">
            <p
              className={`text-xs ${modifyBlocked ? 'text-yellow-400' : 'text-textSecondary'}`}
              role={modifyBlocked ? 'status' : undefined}
            >
              {modifyBlocked
                ? modifyBlocked
                : modifyStep === 'emote'
                ? selectedModifyEmote
                  ? `Selected: ${modifiableEmotes.find(e => e.id === selectedModifyEmote)?.token || 'Unknown'}`
                  : 'Click an emote to select it'
                : selectedModifier
                  ? `Effect: ${MODIFIER_INFO[selectedModifier.modifier_id]?.name || selectedModifier.token}`
                  : 'Choose a modification effect'
              }
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => {
                  if (modifyStep === 'modifier') {
                    setModifyStep('emote');
                    setSelectedModifier(null);
                  } else {
                    setShowModifyEmoteModal(false);
                    setModifyEmoteReward(null);
                    setSelectedModifyEmote(null);
                    setSelectedModifier(null);
                    setModifyStep('emote');
                  }
                }}
                className="px-4 py-2 text-sm font-medium text-textSecondary hover:text-textPrimary transition-colors"
              >
                {modifyStep === 'modifier' ? 'Back' : 'Cancel'}
              </button>
              {modifyStep === 'emote' ? (
                <button
                  onClick={() => setModifyStep('modifier')}
                  disabled={!selectedModifyEmote}
                  className={`
                    px-4 py-2 text-sm font-semibold glass-button transition-all
                    ${selectedModifyEmote
                      ? 'text-accent hover:text-accent-hover active:scale-[0.98]'
                      : 'opacity-50 cursor-not-allowed'
                    }
                  `}
                >
                  Next
                </button>
              ) : (
                <button
                  onClick={handleModifyEmoteConfirm}
                  disabled={!selectedModifier || redeemingId !== null || !modifyAvail?.redeemable}
                  className={`
                    px-4 py-2 text-sm font-semibold glass-button transition-all
                    ${selectedModifier && redeemingId === null && modifyAvail?.redeemable
                      ? 'text-accent hover:text-accent-hover active:scale-[0.98]'
                      : 'opacity-50 cursor-not-allowed'
                    }
                  `}
                >
                  {redeemingId !== null ? 'Unlocking...' : 'Confirm'}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Choose Emote Modal - Single step emote picker */}
      {showChooseEmoteModal && chooseEmoteReward && (
        <div 
          className={`animate-scale-in absolute inset-0 z-50 flex flex-col overflow-hidden`}
        >
          {/* Header */}
          <div className="flex items-center justify-between p-4 border-b border-borderSubtle">
            <div className="flex items-center gap-3">
              <button
                onClick={() => {
                  setShowChooseEmoteModal(false);
                  setChooseEmoteReward(null);
                  setSelectedUnlockEmote(null);
                }}
                className="p-1.5 text-textSecondary hover:text-textPrimary hover:bg-glass rounded-lg transition-all"
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
              <div>
                <h3 className="text-base font-semibold text-textPrimary">Choose an Emote</h3>
                <p className="text-xs text-textSecondary">{chooseEmoteReward.title}</p>
              </div>
            </div>
            <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-accent/20">
              {customPointsIconUrl ? (
                <img src={customPointsIconUrl} alt="" className="w-4 h-4" />
              ) : (
                <ChannelPointsIcon size={14} className="text-accent" />
              )}
              <span className="text-sm font-bold text-accent">
                {chooseEmoteReward.cost.toLocaleString()}
              </span>
            </div>
          </div>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-4 scrollbar-thin">
            {isLoadingUnlockable ? (
              <div className="flex items-center justify-center h-32">
                <div className="animate-spin rounded-full h-8 w-8 border-2 border-accent border-t-transparent" />
              </div>
            ) : unlockableEmotes.length === 0 ? (
              <div className="flex items-center justify-center h-32">
                <p className="text-sm text-textSecondary">No emotes available to unlock</p>
              </div>
            ) : (
              <div className="grid grid-cols-6 gap-1.5">
                {unlockableEmotes.map((emote) => {
                  const imageUrl = `https://static-cdn.jtvnw.net/emoticons/v2/${emote.id}/default/dark/2.0`;
                  const isSelected = selectedUnlockEmote?.id === emote.id;

                  return (
                    <button
                      key={emote.id}
                      onClick={() => setSelectedUnlockEmote(emote)}
                      className={`
                        relative flex flex-col items-center gap-1 p-2 rounded-lg transition-all border border-transparent
                        ${isSelected
                          ? 'bg-surface-active border-border text-accent-neon shadow-inner'
                          : 'hover:bg-surface-hover'
                        }
                      `}
                    >
                      <img
                        src={imageUrl}
                        alt={emote.token}
                        className="w-8 h-8 object-contain"
                        onError={(e) => {
                          e.currentTarget.style.display = 'none';
                        }}
                      />
                      <span className="text-[10px] text-textSecondary truncate w-full text-center">
                        {emote.token}
                      </span>
                      {isSelected && (
                        <div className="absolute top-1 right-1 w-4 h-4 bg-accent rounded-full flex items-center justify-center">
                          <svg className="w-2.5 h-2.5 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={3} d="M5 13l4 4L19 7" />
                          </svg>
                        </div>
                      )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="flex items-center justify-between p-4 border-t border-borderSubtle">
            <p
              className={`text-xs ${chooseBlocked ? 'text-yellow-400' : 'text-textSecondary'}`}
              role={chooseBlocked ? 'status' : undefined}
            >
              {chooseBlocked
                ? chooseBlocked
                : selectedUnlockEmote
                ? `Selected: ${selectedUnlockEmote.token}`
                : 'Click an emote to select it'
              }
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => {
                  setShowChooseEmoteModal(false);
                  setChooseEmoteReward(null);
                  setSelectedUnlockEmote(null);
                }}
                className="px-4 py-2 text-sm font-medium text-textSecondary hover:text-textPrimary hover:bg-glass rounded-lg transition-all"
              >
                Cancel
              </button>
              <button
                onClick={handleChooseEmoteConfirm}
                disabled={!selectedUnlockEmote || redeemingId !== null || !chooseAvail?.redeemable}
                className={`
                  px-4 py-2 text-sm font-semibold glass-button transition-all
                  ${selectedUnlockEmote && redeemingId === null && chooseAvail?.redeemable
                    ? 'text-accent hover:text-accent-hover active:scale-[0.98]'
                    : 'opacity-50 cursor-not-allowed'
                  }
                `}
              >
                {redeemingId !== null ? 'Unlocking...' : 'Unlock'}
              </button>
            </div>
          </div>
        </div>
      )}
    </motion.div>
  );
};

export default ChannelPointsMenu;

