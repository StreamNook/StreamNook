import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { ChannelReward, ChannelRewardsSnapshot, RedemptionResult, RewardAvailability, UnlockedEmote } from '../types';

import { Logger } from '../utils/logger';
import { ChannelPointsIcon } from './ChannelPointsIcon';
import { Tooltip } from './ui/Tooltip';

/** The built-in reward whose text goes out through send_highlighted_message. */
const HIGHLIGHTED_MESSAGE_REWARD = 'SEND_HIGHLIGHTED_MESSAGE';

/** Opening a reward re-reads the list when it is older than this. */
const REWARDS_STALE_MS = 30_000;
/** setTimeout's ceiling; longer cooldowns re-arm after a refresh. */
const MAX_TIMER_MS = 2_147_483_647;

const formatCountdown = (ms: number): string => {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
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

  // Typed-input modal state (Highlight My Message and custom input rewards)
  const [showInputModal, setShowInputModal] = useState(false);
  const [inputText, setInputText] = useState('');
  const [inputReward, setInputReward] = useState<ChannelReward | null>(null);

  // Emote reveal popup state
  const [showEmoteReveal, setShowEmoteReveal] = useState(false);
  const [revealedEmote, setRevealedEmote] = useState<UnlockedEmote | null>(null);

  // Confirmation modal state (for costly redemptions like emote unlocks)
  const [showConfirmModal, setShowConfirmModal] = useState(false);
  const [pendingReward, setPendingReward] = useState<ChannelReward | null>(null);

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

  // A modal holds the reward it opened with; read its current verdict from the
  // latest list. A reward that dropped out of the list cannot be redeemed.
  const liveOf = (reward: ChannelReward): ChannelReward => {
    const live = rewards.find((r) => r.id === reward.id);
    if (live) return live;
    return {
      ...reward,
      availability: { ...reward.availability, redeemable: false, reason: 'disabled', cooldown_until_ms: null },
    };
  };

  /** Plain sentence shown beside a blocked confirm button. */
  const blockedText = (availability: RewardAvailability): string | null => {
    switch (availability.reason) {
      case null:
        return null;
      case 'disabled':
        return 'Turned off by the streamer';
      case 'paused':
        return 'Paused by the streamer';
      case 'out_of_stock':
        return 'Out of stock';
      case 'input_not_supported':
        return 'Needs typed input, not supported yet';
      case 'max_per_stream_reached':
        return 'Limit reached for this stream';
      case 'on_cooldown':
        return availability.cooldown_until_ms !== null
          ? `On cooldown, ${formatCountdown(availability.cooldown_until_ms - nowMs)} left`
          : 'On cooldown';
      case 'not_enough_points':
        return `Not enough ${customPointsName || 'points'}`;
    }
  };

  /** Short badge for a blocked row in the list. */
  const badgeText = (availability: RewardAvailability): string | null => {
    switch (availability.reason) {
      case 'disabled':
        return 'Off';
      case 'paused':
        return 'Paused';
      case 'out_of_stock':
        return 'Out of stock';
      case 'input_not_supported':
        return 'Needs input';
      case 'max_per_stream_reached':
        return 'Limit reached';
      case 'on_cooldown':
        return availability.cooldown_until_ms !== null
          ? `Cooldown ${formatCountdown(availability.cooldown_until_ms - nowMs)}`
          : 'Cooldown';
      // Too few points shows on the red cost pill instead.
      default:
        return null;
    }
  };

  // Close on Escape key
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const handleRedeem = async (reward: ChannelReward) => {
    if (redeemingId) return; // Already redeeming something
    if (!reward.availability.redeemable) {
      useAppStore
        .getState()
        .addToast(blockedText(reward.availability) ?? 'This reward is not available', 'error');
      return;
    }

    // The verdict may have gone stale while the menu sat open (a reward
    // paused, a cooldown started elsewhere). The view below follows the
    // fresh list as soon as it lands.
    void refreshIfStale();

    // Input rewards the backend lets through: Highlight My Message and the
    // streamer's custom rewards that ask for text.
    if (reward.is_user_input_required) {
      setInputReward(reward);
      setInputText('');
      setShowInputModal(true);
      return;
    }

    // Handle random emote unlock - show confirmation first
    const titleLower = reward.title.toLowerCase();
    if (titleLower.includes('random') && titleLower.includes('emote')) {
      setPendingReward(reward);
      setShowConfirmModal(true);
      return;
    }

    // Handle modify single emote - show emote picker
    if (titleLower.includes('modify') && titleLower.includes('emote')) {
      setModifyEmoteReward(reward);
      setSelectedModifyEmote(null);
      setShowModifyEmoteModal(true);
      // Fetch the list of modifiable emotes
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
      return;
    }

    // Handle choose specific emote to unlock - show emote picker
    if ((titleLower.includes('choose') || titleLower.includes('unlock a sub emote')) && titleLower.includes('emote') && !titleLower.includes('modify') && !titleLower.includes('random')) {
      setChooseEmoteReward(reward);
      setSelectedUnlockEmote(null);
      setShowChooseEmoteModal(true);
      // Fetch the list of unlockable emotes (same as modifiable but we just need base emotes)
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
        // Map to simple emote list (just base emotes without modifications)
        setUnlockableEmotes(emotes.map(e => ({ id: e.id, token: e.token })));
      } catch (err) {
        Logger.error('[ChannelPointsMenu] Failed to fetch unlockable emotes:', err);
        useAppStore.getState().addToast('Failed to load emotes', 'error');
      } finally {
        setIsLoadingUnlockable(false);
      }
      return;
    }

    // Standard redemption — route through the confirm modal so the user has a chance
    // to back out before spending points. The actual invoke happens in
    // handleConfirmRedemption, which branches by reward type.
    setPendingReward(reward);
    setShowConfirmModal(true);
  };

  const handleSendInput = async () => {
    if (!inputReward || !inputText.trim()) {
      useAppStore.getState().addToast('Please enter a message', 'error');
      return;
    }
    if (!liveOf(inputReward).availability.redeemable) return;

    const isHighlight = inputReward.reward_type === HIGHLIGHTED_MESSAGE_REWARD;
    setRedeemingId(inputReward.id);
    try {
      const result = isHighlight
        ? await invoke<RedemptionResult>('send_highlighted_message', {
          channelId: channelId,
          message: inputText.trim(),
          cost: inputReward.cost,
        })
        : await invoke<RedemptionResult>('redeem_channel_reward', {
          channelId: channelId,
          rewardId: inputReward.id,
          cost: inputReward.cost,
          title: inputReward.title,
          prompt: inputReward.prompt ?? '',
          textInput: inputText.trim(),
        });

      if (result.success) {
        useAppStore
          .getState()
          .addToast(isHighlight ? 'Highlighted message sent!' : `Redeemed: ${inputReward.title}`, 'success');
        setShowInputModal(false);
        setInputText('');
        setInputReward(null);
        onBalanceUpdate(); // Refresh balance
        void refreshRewards(); // A redeem can start a cooldown or use up stock
      } else {
        useAppStore.getState().addToast(result.error_message || 'Failed to send', 'error');
      }
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Input reward error:', err);
      useAppStore.getState().addToast(typeof err === 'string' ? err : 'Failed to send', 'error');
    } finally {
      setRedeemingId(null);
    }
  };

  const handleConfirmRedemption = async () => {
    if (!pendingReward) return;
    if (!liveOf(pendingReward).availability.redeemable) return;

    const reward = pendingReward;
    const titleLower = reward.title.toLowerCase();
    const isRandomEmote =
      titleLower.includes('random') && titleLower.includes('emote');

    setShowConfirmModal(false);
    setRedeemingId(reward.id);

    try {
      if (isRandomEmote) {
        const result = await invoke<RedemptionResult>('unlock_random_emote', {
          channelId: channelId,
          cost: reward.cost,
        });
        if (result.success) {
          if (result.unlocked_emote) {
            setRevealedEmote(result.unlocked_emote);
            setShowEmoteReveal(true);
          } else {
            useAppStore.getState().addToast('Random emote unlocked!', 'success');
          }
          onBalanceUpdate();
          void refreshRewards();
          if (onEmotesChange) onEmotesChange();
        } else {
          useAppStore
            .getState()
            .addToast(result.error_message || 'Failed to unlock emote', 'error');
        }
      } else {
        const result = await invoke<RedemptionResult>('redeem_channel_reward', {
          channelId: channelId,
          rewardId: reward.id,
          cost: reward.cost,
          title: reward.title,
          prompt: reward.prompt ?? '',
        });
        if (result.success) {
          useAppStore.getState().addToast(`Redeemed: ${reward.title}`, 'success');
          onBalanceUpdate();
          void refreshRewards();
        } else {
          useAppStore
            .getState()
            .addToast(result.error_message || 'Redemption failed', 'error');
        }
      }
    } catch (err) {
      Logger.error('[ChannelPointsMenu] Redemption error:', err);
      useAppStore
        .getState()
        .addToast(typeof err === 'string' ? err : 'Redemption failed', 'error');
    } finally {
      setRedeemingId(null);
      setPendingReward(null);
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

  // Current verdicts for whichever confirm view is open.
  const inputAvail = inputReward ? liveOf(inputReward).availability : null;
  const inputIsHighlight = inputReward?.reward_type === HIGHLIGHTED_MESSAGE_REWARD;
  const pendingAvail = pendingReward ? liveOf(pendingReward).availability : null;
  const modifyAvail = modifyEmoteReward ? liveOf(modifyEmoteReward).availability : null;
  const chooseAvail = chooseEmoteReward ? liveOf(chooseEmoteReward).availability : null;
  const inputBlocked = inputAvail ? blockedText(inputAvail) : null;
  const pendingBlocked = pendingAvail ? blockedText(pendingAvail) : null;
  const modifyBlocked = modifyAvail ? blockedText(modifyAvail) : null;
  const chooseBlocked = chooseAvail ? blockedText(chooseAvail) : null;

  return (
    <div
      ref={menuRef}
      className="glass-panel absolute bottom-full left-0 right-0 mb-2 flex flex-col max-h-[400px] overflow-hidden z-50"
      style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))', backdropFilter: 'blur(24px)', WebkitBackdropFilter: 'blur(24px)' }}
    >
      {/* Header with balance */}
      <div className="px-4 py-3 border-b border-borderSubtle bg-black/20">
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium text-textSecondary">
            {customPointsName || 'Channel Points'}
          </span>
          <Tooltip content="Click to refresh" side="top">
            <button
              type="button"
              onClick={handleRefreshBalance}
              disabled={isRefreshingBalance}
              aria-label={`Refresh ${customPointsName || 'channel points'} balance`}
              className="flex items-center gap-1.5 rounded-md px-1.5 py-0.5 -mr-1.5 transition-colors hover:bg-surface-hover disabled:opacity-60"
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
          <div className="p-2 space-y-1">
            {rewards.map((reward) => {
              const available = reward.availability.redeemable;
              const statusText = badgeText(reward.availability);
              const isRedeeming = redeemingId === reward.id;
              const canAfford = reward.availability.affordable;

              return (
                <button
                  key={reward.id}
                  onClick={() => handleRedeem(reward)}
                  disabled={!available || isRedeeming}
                  className={`
                    w-full flex items-center gap-3 p-2.5 rounded-lg transition-all text-left
                    ${available 
                      ? 'hover:bg-surface-hover cursor-pointer active:scale-[0.99]' 
                      : 'opacity-50 cursor-not-allowed'}
                    ${isRedeeming ? 'animate-pulse bg-accent/10' : ''}
                  `}
                >
                  {/* Reward image/color */}
                  <div 
                    className="w-9 h-9 rounded-lg flex items-center justify-center flex-shrink-0 overflow-hidden"
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

                  {/* Reward info - grows but truncates */}
                  <div className="flex-1 min-w-0 overflow-hidden">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-textPrimary truncate">
                        {reward.title}
                      </span>
                    </div>
                    {reward.prompt && (
                      <p className="text-xs text-textMuted truncate mt-0.5">
                        {reward.prompt}
                      </p>
                    )}
                  </div>

                  {/* Status badge if not available */}
                  {statusText && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-yellow-500/20 text-yellow-400 flex-shrink-0 whitespace-nowrap">
                      {statusText}
                    </span>
                  )}

                  {/* Cost - always visible */}
                  <div className={`
                    flex items-center gap-1 px-2.5 py-1 rounded-full text-xs font-semibold flex-shrink-0
                    ${canAfford 
                      ? 'bg-accent/20 text-accent' 
                      : 'bg-red-500/20 text-red-400'}
                  `}>
                    {customPointsIconUrl ? (
                      <img src={customPointsIconUrl} alt="" className="w-3 h-3" />
                    ) : (
                      <ChannelPointsIcon size={12} />
                    )}
                    {reward.cost >= 1000 
                      ? `${(reward.cost / 1000).toFixed(reward.cost % 1000 === 0 ? 0 : 1)}k`
                      : reward.cost.toLocaleString()}
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </div>

      {/* Typed-input modal: Highlight My Message or a custom input reward */}
      {showInputModal && inputReward && (
        <div 
          className="glass-panel animate-scale-in absolute inset-0 flex flex-col overflow-hidden z-50"
          style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))', backdropFilter: 'blur(24px)', WebkitBackdropFilter: 'blur(24px)' }}
        >
          {/* Modal Header */}
          <div className="px-4 py-3 border-b border-borderSubtle bg-black/20">
            <div className="flex items-center justify-between">
              <span className={`text-sm font-medium truncate ${inputIsHighlight ? 'text-yellow-400' : 'text-textPrimary'}`}>
                {inputIsHighlight ? `✨ ${inputReward.title}` : inputReward.title}
              </span>
              <div className="flex items-center gap-1.5">
                {customPointsIconUrl ? (
                  <img src={customPointsIconUrl} alt="" className="w-4 h-4" />
                ) : (
                  <ChannelPointsIcon size={16} className="text-accent-neon" />
                )}
                <span className="text-base font-bold text-accent-neon">
                  {inputReward.cost.toLocaleString()}
                </span>
              </div>
            </div>
          </div>

          {/* Message Input */}
          <div className="flex-1 min-h-0 p-4 flex flex-col gap-3">
            {!inputIsHighlight && inputReward.prompt && (
              <p className="text-xs text-textSecondary whitespace-pre-wrap break-words max-h-20 overflow-y-auto custom-scrollbar">
                {inputReward.prompt}
              </p>
            )}
            <textarea
              value={inputText}
              onChange={(e) => setInputText(e.target.value)}
              placeholder={inputIsHighlight ? 'Type your highlighted message...' : 'Type your answer...'}
              maxLength={500}
              autoFocus
              className="flex-1 w-full glass-input px-3 py-2 text-sm text-textPrimary placeholder-textMuted resize-none focus:outline-none"
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  handleSendInput();
                }
                if (e.key === 'Escape') {
                  setShowInputModal(false);
                }
              }}
            />
            <div className="flex items-center justify-between">
              {inputBlocked ? (
                <span className="text-xs text-yellow-400" role="status">
                  {inputBlocked}
                </span>
              ) : (
                <span className="text-xs text-textMuted">
                  {inputText.length}/500
                </span>
              )}
              <div className="flex gap-2">
                <button
                  onClick={() => setShowInputModal(false)}
                  className="px-3 py-1.5 text-xs font-medium text-textSecondary hover:text-textPrimary transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleSendInput}
                  disabled={!inputText.trim() || redeemingId === inputReward.id || !inputAvail?.redeemable}
                  className={`
                    px-4 py-1.5 text-xs font-semibold glass-button transition-all
                    ${inputText.trim() && redeemingId !== inputReward.id && inputAvail?.redeemable
                      ? (inputIsHighlight ? 'text-yellow-400 hover:text-yellow-300' : 'text-accent hover:text-accent-neon')
                      : 'opacity-50 cursor-not-allowed'}
                  `}
                >
                  {redeemingId === inputReward.id
                    ? (inputIsHighlight ? 'Sending...' : 'Redeeming...')
                    : (inputIsHighlight ? 'Send Highlighted' : 'Redeem')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Emote Reveal Popup */}
      {showEmoteReveal && revealedEmote && (
        <div 
          className="glass-panel animate-scale-in absolute inset-0 flex flex-col items-center justify-center overflow-hidden z-50"
          style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))', backdropFilter: 'blur(24px)', WebkitBackdropFilter: 'blur(24px)' }}
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

      {/* Confirmation Modal */}
      {showConfirmModal && pendingReward && (
        <div 
          className="glass-panel animate-scale-in absolute inset-0 flex flex-col items-center justify-center overflow-hidden z-50"
          style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))', backdropFilter: 'blur(24px)', WebkitBackdropFilter: 'blur(24px)' }}
        >
          {/* Header */}
          <div className="text-lg font-bold text-textPrimary mb-2">
            Confirm Redemption
          </div>

          {/* Reward Image */}
          {pendingReward.image_url && (
            <img
              src={pendingReward.image_url}
              alt={pendingReward.title}
              className="w-16 h-16 object-cover rounded-lg mb-3"
            />
          )}

          {/* Reward Title */}
          <div className="text-base font-semibold text-textPrimary mb-1">
            {pendingReward.title}
          </div>

          {/* Warning Text — generic; show the streamer's prompt when available */}
          <div className="text-sm text-textMuted text-center px-6 mb-4">
            {pendingReward.prompt
              ? pendingReward.prompt
              : pendingReward.title.toLowerCase().includes('random') &&
                pendingReward.title.toLowerCase().includes('emote')
                ? 'You will receive a random subscriber emote.'
                : 'Spend channel points to redeem this reward.'}
            <br />
            {pendingBlocked ? (
              <span className="text-yellow-400" role="status">{pendingBlocked}</span>
            ) : (
              <span className="text-yellow-400/80">This cannot be undone.</span>
            )}
          </div>

          {/* Cost */}
          <div className="flex items-center gap-2 mb-6">
            <div className="flex items-center gap-1.5 px-4 py-2 rounded-full bg-accent/20">
              {customPointsIconUrl ? (
                <img src={customPointsIconUrl} alt="" className="w-5 h-5" />
              ) : (
                <ChannelPointsIcon size={18} className="text-accent" />
              )}
              <span className="text-lg font-bold text-accent">
                {pendingReward.cost.toLocaleString()}
              </span>
            </div>
          </div>

          {/* Buttons */}
          <div className="flex gap-3">
            <button
              onClick={() => {
                setShowConfirmModal(false);
                setPendingReward(null);
              }}
              className="px-6 py-2 text-sm font-medium text-textSecondary hover:text-textPrimary transition-colors rounded-lg border border-border hover:border-borderLight"
            >
              Cancel
            </button>
            <button
              onClick={handleConfirmRedemption}
              disabled={redeemingId !== null || !pendingAvail?.redeemable}
              className={`
                px-6 py-2 text-sm font-semibold glass-button transition-all
                ${redeemingId === null && pendingAvail?.redeemable
                  ? 'text-accent hover:text-accent-hover active:scale-[0.98]'
                  : 'opacity-50 cursor-not-allowed'}
              `}
            >
              {redeemingId !== null ? 'Unlocking...' : 'Confirm'}
            </button>
          </div>
        </div>
      )}

      {/* Modify Emote Picker Modal */}
      {showModifyEmoteModal && modifyEmoteReward && (
        <div 
          className="glass-panel animate-scale-in absolute inset-0 z-50 flex flex-col overflow-hidden"
          style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))', backdropFilter: 'blur(24px)', WebkitBackdropFilter: 'blur(24px)' }}
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
          style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, var(--glass-under, transparent))', backdropFilter: 'blur(24px)', WebkitBackdropFilter: 'blur(24px)' }}
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

