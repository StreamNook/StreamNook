import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { ChannelReward, ChannelRewardsSnapshot, RedemptionResult, UnlockedEmote } from '../types';

import { Logger } from '../utils/logger';
import { formatCostShort, newTransactionId, rewardKind, rewardStatusText, rewardTakesText } from '../utils/rewardStatus';
import { ChannelPointsIcon } from './ChannelPointsIcon';
import { Tooltip } from './ui/Tooltip';

/** Opening a reward re-reads the list when it is older than this. */
const REWARDS_STALE_MS = 30_000;
/** setTimeout's ceiling; longer cooldowns re-arm after a refresh. */
const MAX_TIMER_MS = 2_147_483_647;
/** Room the detail view and the emote pickers need, however short the list. */
const OVERLAY_MIN_HEIGHT = 340;
/** Shared with Twitch's own limit for a reward's typed text. */
const INPUT_MAX = 500;

const panelGlass: React.CSSProperties = {
  backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))',
  backdropFilter: 'blur(24px)',
  WebkitBackdropFilter: 'blur(24px)',
};

/** A status as a dot and words, never a chip. */
const StatusLine: React.FC<{ text: string; className?: string; title?: string }> = ({ text, className = '', title }) => (
  <span className={`flex items-start gap-1.5 min-w-0 text-xs leading-snug text-textSecondary ${className}`} role="status" title={title}>
    <span className="w-1.5 h-1.5 mt-[5px] rounded-full bg-yellow-400 flex-shrink-0" aria-hidden="true" />
    <span className="line-clamp-2 break-words">{text}</span>
  </span>
);

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
}) => {
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
  // One transaction id per redeem attempt: kept while the same reward and text
  // are retried, so Twitch recognises a retry after a lost answer instead of
  // posting and charging twice. A success, closing the view or new text drops it.
  const transactionRef = useRef<{ key: string; id: string } | null>(null);
  const transactionFor = (rewardId: string, text: string): string => {
    const key = `${rewardId}\n${text}`;
    if (transactionRef.current?.key !== key) {
      transactionRef.current = { key, id: newTransactionId() };
    }
    return transactionRef.current.id;
  };

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
        if (initial) {
          setError(typeof err === 'string' ? err : 'Failed to load rewards');
          setIsLoading(false);
        }
      },
    );
  }, [channelLogin]);

  // Fetch rewards when menu opens
  useEffect(() => {
    const seqRef = fetchSeqRef;
    void refreshRewards(true);
    // Unmounting (or a channel switch) voids any call still in flight.
    return () => { seqRef.current++; };
  }, [refreshRewards]);

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
    transactionRef.current = null;
  }, []);

  // Escape backs out of the open view first, then closes the menu.
  const detailOpen = detailReward !== null;
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (detailOpen) {
        closeDetail();
        return;
      }
      onClose();
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose, closeDetail, detailOpen]);

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
    const kind = rewardKind(reward);

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
    if (rewardTakesText(kind) && !text) return;

    setRedeemingId(reward.id);
    // Rust treats Twitch's "already committed" answer to a reused id as sent.
    const transactionId = transactionFor(reward.id, text);
    try {
      let result: RedemptionResult;
      if (kind === 'highlight') {
        result = await invoke<RedemptionResult>('send_highlighted_message', {
          channelId: channelId,
          message: text,
          cost: reward.cost,
          transactionId: transactionId,
        });
      } else if (kind === 'sub_message') {
        result = await invoke<RedemptionResult>('send_sub_mode_message', {
          channelId: channelId,
          channelLogin: channelLogin,
          message: text,
          cost: reward.cost,
          transactionId: transactionId,
        });
      } else if (kind === 'random_emote') {
        result = await invoke<RedemptionResult>('unlock_random_emote', {
          channelId: channelId,
          cost: reward.cost,
        });
      } else {
        result = await invoke<RedemptionResult>('redeem_channel_reward', {
          channelId: channelId,
          rewardId: reward.id,
          cost: reward.cost,
          title: reward.title,
          prompt: reward.prompt ?? '',
          transactionId: transactionId,
          ...(kind === 'input' ? { textInput: text } : {}),
        });
      }

      if (result.success) {
        closeDetail();
        if (kind === 'random_emote') {
          if (result.unlocked_emote) {
            setRevealedEmote(result.unlocked_emote);
            setShowEmoteReveal(true);
          } else {
            useAppStore.getState().addToast('Random emote unlocked!', 'success');
          }
          if (onEmotesChange) onEmotesChange();
        } else {
          useAppStore
            .getState()
            .addToast(
              kind === 'highlight' ? 'Highlighted message sent!' : kind === 'sub_message' ? 'Message sent' : `Redeemed: ${reward.title}`,
              'success',
            );
        }
        onBalanceUpdate();
        void refreshRewards(); // A redeem can start a cooldown or use up stock
      } else {
        useAppStore.getState().addToast(result.error_message || 'Redemption failed', 'error');
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
      const result = await invoke<RedemptionResult>('unlock_modified_emote', {
        channelId: channelId,
        emoteId: finalEmoteId,
        cost: modifyEmoteReward.cost,
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
        useAppStore.getState().addToast(result.error_message || 'Failed to modify emote', 'error');
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
      const result = await invoke<RedemptionResult>('unlock_chosen_emote', {
        channelId: channelId,
        emoteId: selectedUnlockEmote.id,
        cost: chooseEmoteReward.cost,
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
        useAppStore.getState().addToast(result.error_message || 'Failed to unlock emote', 'error');
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
  const detailKind = detail ? rewardKind(detail) : null;
  const detailTakesText = detailKind !== null && rewardTakesText(detailKind);
  const detailStatus = detail ? rewardStatusText(detail.availability, nowMs, 'detail') : null;
  const detailAbout = detail ? (detail.prompt?.trim() || detail.description?.trim() || null) : null;
  const detailBusy = detail !== null && redeemingId === detail.id;
  const detailCanAct = detail !== null
    && detail.availability.redeemable
    && redeemingId === null
    && (!detailTakesText || inputText.trim().length > 0);
  const detailActionLabel = detailBusy
    ? (detailKind === 'highlight' || detailKind === 'sub_message' ? 'Sending...' : 'Redeeming...')
    : detailKind === 'highlight'
      ? 'Send highlighted'
      : detailKind === 'sub_message'
        ? 'Send'
      : detailKind === 'choose_emote' || detailKind === 'modify_emote'
        ? 'Choose emote'
        : 'Redeem';
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

  return (
    <div
      ref={menuRef}
      className="glass-panel absolute bottom-full left-0 right-0 mb-2 flex flex-col max-h-[400px] overflow-hidden z-50"
      style={{ ...panelGlass, minHeight: overlayOpen ? OVERLAY_MIN_HEIGHT : undefined }}
    >
      {/* Header with balance */}
      <div className="px-4 py-3 border-b border-borderSubtle bg-black/20">
        <div className="flex items-center justify-between gap-2">
          <span className="text-sm font-medium text-textSecondary truncate">
            {customPointsName || 'Channel Points'}
          </span>
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
                {currentBalance?.toLocaleString() ?? '--'}
              </span>
            </button>
          </Tooltip>
        </div>
      </div>

      {/* Content */}
      <div className="overflow-y-auto max-h-[300px] custom-scrollbar">
        {isLoading ? (
          <div className="flex items-center justify-center py-8">
            <div className="animate-spin w-6 h-6 border-2 border-accent border-t-transparent rounded-full" />
          </div>
        ) : error ? (
          <div className="px-4 py-6 text-center">
            <p className="text-textSecondary text-sm">{error}</p>
            <button
              onClick={retryRewards}
              className="mt-2 text-xs text-accent hover:underline"
            >
              Try again
            </button>
          </div>
        ) : rewards.length === 0 ? (
          <div className="px-4 py-6 text-center">
            <p className="text-textSecondary text-sm">No rewards available</p>
            <p className="text-textMuted text-xs mt-1">
              This channel hasn't set up any rewards yet
            </p>
          </div>
        ) : (
          <div className="p-2 space-y-0.5">
            {rewards.map((reward) => {
              const available = reward.availability.redeemable;
              const status = rewardStatusText(reward.availability, nowMs, 'label');
              const about = reward.prompt?.trim() || reward.description?.trim() || null;
              const isRedeeming = redeemingId === reward.id;
              const canAfford = reward.availability.affordable;

              return (
                <button
                  key={reward.id}
                  type="button"
                  onClick={() => openDetail(reward)}
                  aria-label={status ? `${reward.title}, ${status}` : reward.title}
                  className={`
                    w-full flex items-center gap-2 px-2 py-2 rounded-lg transition-colors text-left
                    hover:bg-surface-hover cursor-pointer
                    ${isRedeeming ? 'animate-pulse bg-accent/10' : ''}
                  `}
                >
                  {/* Reward image/color */}
                  <div
                    className={`w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0 overflow-hidden ${available ? '' : 'opacity-50 grayscale'}`}
                    style={{ backgroundColor: reward.background_color || '#9147FF' }}
                  >
                    {reward.image_url ? (
                      <img
                        src={reward.image_url}
                        alt=""
                        className="w-full h-full object-cover"
                        onError={(e) => {
                          e.currentTarget.style.display = 'none';
                        }}
                      />
                    ) : (
                      <ChannelPointsIcon size={18} className="text-white/80" />
                    )}
                  </div>

                  {/* Full name on up to two lines, then why it is blocked or what it is */}
                  <div className="flex-1 min-w-0">
                    <span
                      className={`text-[13px] font-medium leading-snug line-clamp-2 break-words ${available ? 'text-textPrimary' : 'text-textSecondary'}`}
                      title={reward.title}
                    >
                      {reward.title}
                    </span>
                    {status ? (
                      <StatusLine text={status} className="mt-0.5" title={reward.availability.detail ?? undefined} />
                    ) : about ? (
                      <p className="text-xs text-textMuted truncate mt-0.5">{about}</p>
                    ) : null}
                  </div>

                  {/* Cost - always visible */}
                  <div className={`
                    flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-semibold flex-shrink-0 tabular-nums
                    ${canAfford
                      ? (available ? 'bg-accent/20 text-accent' : 'bg-white/5 text-textSecondary')
                      : 'bg-red-500/15 text-red-400'}
                  `}>
                    {pointsGlyph(12)}
                    {formatCostShort(reward.cost)}
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </div>

      {/* Reward detail: the full name, what it does, why it is blocked, and
          the text box for rewards that ask for one */}
      {detail && (
        <div
          className="glass-panel animate-scale-in absolute inset-0 flex flex-col overflow-hidden z-50"
          style={panelGlass}
          role="dialog"
          aria-label={detail.title}
        >
          <div className="flex items-center justify-between gap-2 px-2 py-2 border-b border-borderSubtle bg-black/20">
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
            <div className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full text-sm font-bold tabular-nums ${detail.availability.affordable ? 'bg-accent/20 text-accent' : 'bg-red-500/15 text-red-400'}`}>
              {pointsGlyph(14)}
              {detail.cost.toLocaleString()}
            </div>
          </div>

          <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-4 pt-3 pb-2 flex flex-col gap-2.5">
            <div className="flex items-center gap-3">
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
                <span className="w-1.5 h-1.5 mt-[5px] rounded-full bg-yellow-400 flex-shrink-0" aria-hidden="true" />
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
                className="w-full min-h-[72px] glass-input px-3 py-2 text-sm text-textPrimary placeholder-textMuted resize-none focus:outline-none"
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

          <div className="flex items-center justify-between gap-2 px-4 py-2.5 border-t border-borderSubtle">
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
                flex-shrink-0 px-4 py-1.5 text-xs font-semibold glass-button transition-all whitespace-nowrap
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
          className="glass-panel animate-scale-in absolute inset-0 flex flex-col items-center justify-center overflow-hidden z-50"
          style={panelGlass}
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
          className="glass-panel animate-scale-in absolute inset-0 z-50 flex flex-col overflow-hidden"
          style={panelGlass}
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
          className="glass-panel animate-scale-in absolute inset-0 z-50 flex flex-col overflow-hidden"
          style={panelGlass}
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
    </div>
  );
};

export default ChannelPointsMenu;

