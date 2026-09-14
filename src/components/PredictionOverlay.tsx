import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { Trophy, Users, Hourglass, PartyPopper, RefreshCw, CheckCircle2, XCircle } from 'lucide-react';
import { ChannelPointsIcon } from './ChannelPointsIcon';
import { OverlayBanner } from './chat/OverlayBanner';
import { useAppStore } from '../stores/AppStore';
import { useChannelEmotes } from '../stores/chatConnectionStore';
import { buildEmoteNameMap, EmoteText } from '../utils/emoteText';

import { Logger } from '../utils/logger';
interface PredictionOutcome {
  id: string;
  title: string;
  color: string;
  total_points: number;
  total_users: number;
}

interface PredictionData {
  channel_id: string;
  prediction_id: string;
  title: string;
  outcomes: PredictionOutcome[];
  prediction_window_seconds: number;
  created_at: string;
  status: string;
  winning_outcome_id?: string;
}

interface PredictionOverlayProps {
  channelId?: string;
  channelLogin?: string;
}

const PredictionOverlay = ({ channelId, channelLogin }: PredictionOverlayProps) => {
  const [activePrediction, setActivePrediction] = useState<PredictionData | null>(null);
  const [selectedOutcome, setSelectedOutcome] = useState<string | null>(null);
  const [betAmount, setBetAmount] = useState<number>(10);
  const [betAmountInput, setBetAmountInput] = useState<string>('10');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [timeRemaining, setTimeRemaining] = useState<number>(0);
  const [isLocked, setIsLocked] = useState(false);
  const [channelPoints, setChannelPoints] = useState<number | null>(null);
  const [isExpanded, setIsExpanded] = useState(true);
  // The viewer's own stake in the active prediction, or null when they have
  // none THAT WE KNOW OF. Twitch does not replay a stake placed before this
  // app was listening, so null means "unknown or none", never "definitely
  // zero", and the UI must not claim otherwise.
  const [selfStake, setSelfStake] = useState<{ outcomeId: string; points: number } | null>(null);
  const [resolutionState, setResolutionState] = useState<'none' | 'pending' | 'win' | 'loss' | 'refund' | 'announced'>('none');
  const [winningOutcomeId, setWinningOutcomeId] = useState<string | null>(null);
  // Custom channel points icon URL (e.g., custom lips icon for Hamlinz's "Kisses")
  const [customPointsIconUrl, setCustomPointsIconUrl] = useState<string | null>(null);

  // Refs to track latest values for use in event listeners (avoids stale closures)
  const selfStakeRef = useRef(selfStake);
  const selectedOutcomeRef = useRef(selectedOutcome);

  // Keep refs in sync with state
  useEffect(() => {
    selfStakeRef.current = selfStake;
  }, [selfStake]);

  useEffect(() => {
    selectedOutcomeRef.current = selectedOutcome;
  }, [selectedOutcome]);

  const { addToast, currentStream } = useAppStore();

  // Get current channel ID from props or from currentStream
  const currentChannelId = channelId || currentStream?.user_id;
  const currentChannelLogin = channelLogin || currentStream?.user_login;

  // Outcome titles are plain GQL/PubSub text with no emote ranges, so match
  // emote names against the channel's set the same way pinned messages do.
  const channelEmotes = useChannelEmotes(currentChannelLogin, currentChannelId, 'twitch');
  const emoteMap = useMemo(() => buildEmoteNameMap(channelEmotes), [channelEmotes]);

  // Debug log on mount and when channel changes
  useEffect(() => {
    Logger.debug('[Prediction] PredictionOverlay mounted/updated');
    Logger.debug('[Prediction] Watching for channel:', {
      channelId: currentChannelId || 'NOT SET',
      channelLogin: currentChannelLogin || 'NOT SET',
      fromProps: { channelId, channelLogin },
      fromStore: { user_id: currentStream?.user_id, user_login: currentStream?.user_login }
    });
  }, [currentChannelId, currentChannelLogin, channelId, channelLogin, currentStream]);

  // Fetch active prediction on mount/channel change (for late-joiners)
  useEffect(() => {
    const fetchActivePrediction = async () => {
      if (!currentChannelLogin) return;
      
      Logger.debug('[Prediction] Checking for active prediction on channel:', currentChannelLogin);
      
      try {
        const result = await invoke<PredictionData | null>('get_active_prediction', {
          channelLogin: currentChannelLogin
        });
        
        if (result) {
          Logger.debug('[Prediction] Found active prediction on mount:', result);
          
          // Only set if we don't already have this prediction active
          if (!activePrediction || activePrediction.prediction_id !== result.prediction_id) {
            setActivePrediction(result);
            
            const isFinished = result.status === 'LOCKED' || result.status === 'RESOLVED' || result.status === 'RESOLVE_PENDING';
            setIsLocked(isFinished);
            setIsExpanded(true);
            
            // Handle specific closed states immediately on mount
            if (result.status === 'RESOLVED') {
                setResolutionState('announced'); // User late-joined, we don't know their bet, just announce it
                setWinningOutcomeId(result.winning_outcome_id || null);
                
                // Keep the resolution banner visible briefly then dismiss it (similar to pubsub behavior)
                setTimeout(() => {
                    setActivePrediction(null);
                    setSelectedOutcome(null);
                    setSelfStake(null);
                    setResolutionState('none');
                    setWinningOutcomeId(null);
                }, 5000);
            } else if (result.status === 'RESOLVE_PENDING') {
                setResolutionState('pending');
            } else {
                setResolutionState('none');
            }

            // Auto-select first outcome if prediction is still active (not locked)
            setSelectedOutcome(result.status === 'ACTIVE' && result.outcomes?.length > 0 ? result.outcomes[0].id : null);
            setSelfStake(null);
            
            // Calculate remaining time if prediction is still ACTIVE
            if (result.status === 'ACTIVE' && result.created_at) {
              const createdAt = new Date(result.created_at).getTime();
              const elapsed = Math.floor((Date.now() - createdAt) / 1000);
              const remaining = Math.max(0, result.prediction_window_seconds - elapsed);
              setTimeRemaining(remaining);
              
              if (remaining <= 0) {
                setIsLocked(true);
              }
            } else {
              setTimeRemaining(0);
            }
          }
        } else {
          Logger.debug('[Prediction] No active prediction found on channel');
        }
      } catch (err) {
        Logger.warn('[Prediction] Failed to fetch active prediction:', err);
      }
    };
    
    // Reset state when channel changes
    setActivePrediction(null);
    setSelectedOutcome(null);
    setSelfStake(null);
    setResolutionState('none');
    setWinningOutcomeId(null);
    setChannelPoints(null);
    setCustomPointsIconUrl(null);
    
    // Fetch active prediction for the new channel
    fetchActivePrediction();
  }, [currentChannelLogin]);

  // Fetch channel points when prediction becomes active
  const fetchChannelPoints = useCallback(async () => {
    Logger.debug('[Prediction] fetchChannelPoints called with:', { currentChannelLogin, currentChannelId });
    
    // First try by channel login if available
    if (currentChannelLogin) {
      Logger.debug('[Prediction] Fetching channel points by login:', currentChannelLogin);
      
      try {
        const result = await invoke<any>('get_channel_points_for_channel', {
          channelLogin: currentChannelLogin
        });
        
        Logger.debug('[Prediction] Channel points result:', JSON.stringify(result, null, 2));
        
        // Use the correct path: data.user.channel.self.communityPoints.balance
        const balance = result?.data?.user?.channel?.self?.communityPoints?.balance;
        
        // Extract custom points icon URL
        const customIconUrl = result?.data?.user?.channel?.communityPointsSettings?.image?.url;
        if (customIconUrl) {
          Logger.debug('[Prediction] Got custom points icon:', customIconUrl);
          setCustomPointsIconUrl(customIconUrl);
        } else {
          setCustomPointsIconUrl(null);
        }
        
        if (typeof balance === 'number') {
          Logger.debug('[Prediction] Setting channel points to:', balance);
          setChannelPoints(balance);
          return; // Success!
        }
      } catch (err) {
        Logger.error('[Prediction] Failed to fetch by login:', err);
      }
    }
    
    // Fallback: try by channel ID
    if (currentChannelId) {
      Logger.debug('[Prediction] Trying fallback: get_channel_points_balance with ID:', currentChannelId);
      
      try {
        const result = await invoke<any>('get_channel_points_balance', {
          channelId: currentChannelId
        });
        
        Logger.debug('[Prediction] get_channel_points_balance result:', result);
        
        const balance = result?.balance || result?.points;
        if (typeof balance === 'number') {
          Logger.debug('[Prediction] Setting channel points from fallback:', balance);
          setChannelPoints(balance);
          return;
        }
      } catch (err) {
        Logger.error('[Prediction] Fallback also failed:', err);
      }
    }
    
    Logger.debug('[Prediction] Could not fetch channel points with any method');
  }, [currentChannelLogin, currentChannelId]);

  // Listen for channel points updates from backend events
  useEffect(() => {
    // Listen for points spent (includes new balance)
    const unlistenSpent = listen<{ channel_id: string; points: number; balance: number }>('channel-points-spent', (event) => {
      if (currentChannelId && event.payload.channel_id === currentChannelId) {
        Logger.debug('[Prediction] Points spent event - new balance:', event.payload.balance);
        setChannelPoints(event.payload.balance);
      }
    });

    // Listen for points earned (includes new balance)
    const unlistenEarned = listen<{ channel_id: string; points: number; balance: number }>('channel-points-earned', (event) => {
      if (currentChannelId && event.payload.channel_id === currentChannelId) {
        Logger.debug('[Prediction] Points earned event - new balance:', event.payload.balance);
        setChannelPoints(event.payload.balance);
      }
    });

    return () => {
      // Tauri's unlisten throws synchronously if the registry entry is already
      // gone (e.g. teardown after the webview reset), surfacing as an uncaught
      // rejection. Swallow it: the listener is already removed.
      unlistenSpent.then(fn => fn()).catch(() => {});
      unlistenEarned.then(fn => fn()).catch(() => {});
    };
  }, [currentChannelId]);

  // Listen for prediction events
  useEffect(() => {
    Logger.debug('[Prediction] Setting up event listeners...');
    
    const unlistenCreated = listen<PredictionData>('prediction-created', (event) => {
      const prediction = event.payload;
      Logger.debug('[Prediction] Received prediction-created event:', {
        eventChannelId: prediction.channel_id,
        currentChannelId: currentChannelId,
        match: prediction.channel_id === currentChannelId,
        title: prediction.title
      });
      
      // Only show if this prediction is for the current channel we're watching
      if (currentChannelId && prediction.channel_id === currentChannelId) {
        Logger.debug('[Prediction] Prediction MATCHES current channel! Showing overlay.');
        setActivePrediction(prediction);
        setTimeRemaining(prediction.prediction_window_seconds);
        setIsLocked(false);
        // Auto-select first outcome for immediate betting
        setSelectedOutcome(prediction.outcomes?.length > 0 ? prediction.outcomes[0].id : null);
        setSelfStake(null);
        setIsExpanded(true);
        fetchChannelPoints();
      }
    });

    const unlistenUpdated = listen<PredictionData & { winning_outcome_id?: string }>('prediction-updated', (event) => {
      const prediction = event.payload;
      Logger.debug('[Prediction] Received prediction-updated event:', {
        eventChannelId: prediction.channel_id,
        currentChannelId: currentChannelId,
        match: prediction.channel_id === currentChannelId,
        status: prediction.status,
        hasActivePrediction: !!activePrediction
      });
      
      // If we don't have an active prediction but this is for our channel and ACTIVE, initialize it
      // This handles the case where user starts watching after prediction was created
      if (currentChannelId && prediction.channel_id === currentChannelId) {
        if (!activePrediction && (prediction.status === 'ACTIVE' || prediction.status === 'LOCKED')) {
          Logger.debug('[Prediction] Late-joining prediction! Initializing overlay from update event.');
          setActivePrediction(prediction);
          setTimeRemaining(prediction.prediction_window_seconds || 60);
          setIsLocked(prediction.status === 'LOCKED');
          // Auto-select first outcome if not locked
          setSelectedOutcome(prediction.status === 'ACTIVE' && prediction.outcomes?.length > 0 ? prediction.outcomes[0].id : null);
          setSelfStake(null);
          setIsExpanded(true);
          setResolutionState('none');
          fetchChannelPoints();
        } else if (activePrediction?.prediction_id === prediction.prediction_id) {
          Logger.debug('[Prediction] Prediction updated:', prediction);
          setActivePrediction(prev => prev ? { ...prev, ...prediction } : null);
          
          if (prediction.status === 'LOCKED') {
            setIsLocked(true);
          }
          
          // Handle resolution states
          if (prediction.status === 'RESOLVE_PENDING') {
            Logger.debug('[Prediction] Prediction is being resolved...');
            setResolutionState('pending');
          } else if (prediction.status === 'RESOLVED') {
            Logger.debug('[Prediction] Prediction RESOLVED! winning_outcome_id:', prediction.winning_outcome_id);
            
            // Use the winning_outcome_id from the event payload
            const winningId = prediction.winning_outcome_id;
            
            if (winningId) {
              const winningOutcome = prediction.outcomes?.find(o => o.id === winningId);
              setWinningOutcomeId(winningId);
              
              // Use refs to get latest values (avoids stale closure issue)
              const userBet = selfStakeRef.current !== null;
              const userSelectedOutcome = selectedOutcomeRef.current;
              
              Logger.debug('[Prediction] Resolution check:', {
                winningId,
                userBet,
                userSelectedOutcome,
                didWin: userSelectedOutcome === winningId
              });
              
              // Did user win or lose?
              if (userBet && userSelectedOutcome) {
                if (userSelectedOutcome === winningId) {
                  setResolutionState('win');
                  addToast(`You WON! "${winningOutcome?.title || 'Unknown'}" was correct!`, 'success');
                } else {
                  setResolutionState('loss');
                  addToast(`You lost. "${winningOutcome?.title || 'Unknown'}" was the winner.`, 'error');
                }
              } else {
                // User didn't bet, just show neutral result announcement
                setResolutionState('announced');
                addToast(`Prediction ended! Winner: ${winningOutcome?.title || 'Unknown'}`, 'success');
              }
            } else {
              // No winner ID - prediction was cancelled/refunded
              Logger.debug('[Prediction] No winning_outcome_id - prediction was refunded');
              setResolutionState('refund');
              addToast(`Prediction refunded`, 'info');
            }
            
            // Clear overlay after showing result
            setTimeout(() => {
              setActivePrediction(null);
              setSelectedOutcome(null);
              setSelfStake(null);
              setResolutionState('none');
              setWinningOutcomeId(null);
            }, 4000);
          } else if (prediction.status === 'CANCELED') {
            Logger.debug('[Prediction] Prediction CANCELED');
            setResolutionState('refund');
            addToast(`Prediction cancelled - points refunded`, 'info');
            
            setTimeout(() => {
              setActivePrediction(null);
              setSelectedOutcome(null);
              setSelfStake(null);
              setResolutionState('none');
            }, 3000);
          }
        }
      }
    });

    const unlistenLocked = listen<{ channel_id: string; prediction_id: string }>('prediction-locked', (event) => {
      if (currentChannelId && event.payload.channel_id === currentChannelId && activePrediction?.prediction_id === event.payload.prediction_id) {
        Logger.debug('[Prediction] Prediction locked');
        setIsLocked(true);
      }
    });

    const unlistenEnded = listen<{ channel_id: string; prediction_id: string; winning_outcome_id?: string }>('prediction-ended', (event) => {
      if (currentChannelId && event.payload.channel_id === currentChannelId && activePrediction?.prediction_id === event.payload.prediction_id) {
        Logger.debug('[Prediction] Prediction ended, winner:', event.payload.winning_outcome_id);
        
        // Show result briefly before closing
        if (event.payload.winning_outcome_id) {
          const winner = activePrediction?.outcomes.find(o => o.id === event.payload.winning_outcome_id);
          if (winner) {
            addToast(`Prediction ended! Winner: ${winner.title}`, 'success');
          }
        }
        
        // Close the overlay after a short delay
        setTimeout(() => {
          setActivePrediction(null);
          setSelectedOutcome(null);
          setSelfStake(null);
        }, 2000);
      }
    });

    // The viewer's own stake, straight from Rust. `points` is the running total
    // rather than a delta, so this replaces the optimistic value rather than
    // adding to it, which also corrects a failed or partial local write.
    const unlistenSelf = listen<{
      kind: string;
      channel_id: string;
      event_id: string;
      outcome_id: string;
      points: number;
      result_type: string | null;
    }>('prediction-self-updated', (event) => {
      const p = event.payload;
      if (!currentChannelId || p.channel_id !== currentChannelId) return;
      if (p.result_type) return; // resolution is handled by prediction-ended
      Logger.debug(`[Prediction] self stake ${p.points} on ${p.outcome_id}`);
      setSelfStake({ outcomeId: p.outcome_id, points: p.points });
      setSelectedOutcome(p.outcome_id);
      fetchChannelPoints();
    });

    return () => {
      // See note above: a teardown unlisten can throw if the registry entry is
      // already gone; swallow it so it doesn't surface as an uncaught rejection.
      unlistenCreated.then(fn => fn()).catch(() => {});
      unlistenUpdated.then(fn => fn()).catch(() => {});
      unlistenLocked.then(fn => fn()).catch(() => {});
      unlistenEnded.then(fn => fn()).catch(() => {});
      unlistenSelf.then(fn => fn()).catch(() => {});
    };
  }, [currentChannelId, activePrediction?.prediction_id, addToast, fetchChannelPoints]);

  // Countdown timer
  useEffect(() => {
    if (!activePrediction || isLocked || timeRemaining <= 0) return;

    const timer = setInterval(() => {
      setTimeRemaining(prev => {
        if (prev <= 1) {
          setIsLocked(true);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => clearInterval(timer);
  }, [activePrediction, isLocked, timeRemaining]);

  // Handle placing a prediction
  // Twitch keeps ONE prediction per user per event and adds to it, so placing
  // again on the same outcome tops the stake up. A different outcome is not a
  // second bet, it is rejected, so the UI locks the choice once a stake exists.
  const handlePlacePrediction = async () => {
    if (!activePrediction || !selectedOutcome || isSubmitting || isLocked) return;
    if (selfStake && selfStake.outcomeId !== selectedOutcome) return;

    setIsSubmitting(true);

    try {
      await invoke('place_prediction', {
        eventId: activePrediction.prediction_id,
        outcomeId: selectedOutcome,
        points: betAmount,
        channelId: currentChannelId
      });

      const selectedOutcomeTitle = activePrediction.outcomes.find(o => o.id === selectedOutcome)?.title;
      const topUp = selfStake !== null;
      addToast(
        topUp
          ? `Added ${betAmount} points to "${selectedOutcomeTitle}"`
          : `Prediction placed! ${betAmount} points on "${selectedOutcomeTitle}"`,
        'success'
      );
      // Optimistic; the predictions-user-v1 event reconciles it with the real
      // total a moment later.
      setSelfStake((prev) => ({
        outcomeId: selectedOutcome,
        points: (prev?.points ?? 0) + betAmount,
      }));
      
      // Refresh channel points
      fetchChannelPoints();
    } catch (err: any) {
      Logger.error('[Prediction] Failed to place prediction:', err);
      addToast(`Failed to place prediction: ${err}`, 'error');
    } finally {
      setIsSubmitting(false);
    }
  };

  // Format time remaining
  const formatTime = (seconds: number) => {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  };

  // Get color class for outcome
  const getOutcomeColor = (color: string, isSelected: boolean) => {
    const baseColors: Record<string, string> = {
      'BLUE': isSelected ? 'bg-highlight-blue border-white/25' : 'bg-highlight-blue/20 border-highlight-blue/50 hover:bg-highlight-blue/30',
      'PINK': isSelected ? 'bg-highlight-pink border-white/25' : 'bg-highlight-pink/20 border-highlight-pink/50 hover:bg-highlight-pink/30',
    };
    return baseColors[color] || (isSelected ? 'bg-highlight-purple border-white/25' : 'bg-highlight-purple/20 border-highlight-purple/50 hover:bg-highlight-purple/30');
  };

  // Calculate percentage for outcome
  const getOutcomePercentage = (outcome: PredictionOutcome) => {
    const totalPoints = activePrediction?.outcomes.reduce((sum, o) => sum + o.total_points, 0) || 0;
    if (totalPoints === 0) return 50; // Default to 50% if no bets yet
    return Math.round((outcome.total_points / totalPoints) * 100);
  };

  // Don't render if no active prediction
  if (!activePrediction) return null;

  return (
    <OverlayBanner
      icon={<Trophy className="w-4 h-4 text-accent" />}
      title={activePrediction.title}
      isExpanded={isExpanded}
      onToggleExpanded={() => setIsExpanded(!isExpanded)}
      badges={
        <>
          {channelPoints !== null && (
            <div className="flex items-center gap-1 px-1.5 py-1 bg-accent/20 border border-accent/40 rounded-md">
              {customPointsIconUrl ? (
                <img src={customPointsIconUrl} alt="points" className="w-3 h-3" />
              ) : (
                <ChannelPointsIcon className="text-accent" size={12} />
              )}
              <span className="text-xs font-bold text-accent">
                {channelPoints.toLocaleString()}
              </span>
            </div>
          )}
          {!isLocked ? (
            <span className="text-xs font-mono font-bold text-warning bg-warning/20 border border-warning/40 px-1.5 py-1 rounded-md">
              {formatTime(timeRemaining)}
            </span>
          ) : (
            <span className="text-xs font-medium text-error bg-error/20 border border-error/40 px-1.5 py-1 rounded-md">
              Locked
            </span>
          )}
        </>
      }
    >
      {/* Outcomes */}
      <div className="p-3 space-y-2">
        {activePrediction.outcomes.map((outcome) => {
          const percentage = getOutcomePercentage(outcome);
          const isSelected = selectedOutcome === outcome.id;
          
          return (
            <button
              key={outcome.id}
              onClick={() => !isLocked && !selfStake && setSelectedOutcome(outcome.id)}
              disabled={isLocked || !!selfStake}
              className={`w-full relative p-2.5 rounded-lg border transition-all ${
                getOutcomeColor(outcome.color, isSelected)
              } ${(isLocked || !!selfStake) ? 'cursor-not-allowed opacity-70' : 'cursor-pointer'}`}
            >
              {/* Background progress bar */}
              <div 
                className={`absolute inset-0 rounded-md opacity-30 ${
                  outcome.color === 'BLUE' ? 'bg-highlight-blue' : outcome.color === 'PINK' ? 'bg-highlight-pink' : 'bg-highlight-purple'
                }`}
                style={{ width: `${percentage}%` }}
              />
              
              <div className="relative flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {isSelected && (!isLocked || !!selfStake) && (
                    <div className="w-4 h-4 rounded-full bg-white/30 flex items-center justify-center">
                      <div className="w-2.5 h-2.5 rounded-full bg-white" />
                    </div>
                  )}
                  <span className="font-semibold text-white text-sm">
                    <EmoteText text={outcome.title} emoteMap={emoteMap} keyPrefix={`pred-${outcome.id}`} />
                  </span>
                </div>
                <div className="flex items-center gap-3 text-xs text-white/90">
                  <span className="flex items-center gap-1">
                    <Users className="w-3 h-3" />
                    {outcome.total_users}
                  </span>
                  <span className="flex items-center gap-1">
                    {customPointsIconUrl ? (
                      <img src={customPointsIconUrl} alt="points" className="w-3 h-3" />
                    ) : (
                      <ChannelPointsIcon size={12} className="text-white/90" />
                    )}
                    {outcome.total_points.toLocaleString()}
                  </span>
                  <span className="font-bold text-sm">{percentage}%</span>
                </div>
              </div>
            </button>
          );
        })}
      </div>

      {/* Bet amount and action. Stays available while the window is open so a
          stake can be topped up; the outcome is fixed once one exists. */}
      {!isLocked && (
        <div className="px-3 pb-3">
          {/* Bet Input Row */}
          <div className="flex items-center gap-2 p-2 bg-backgroundSecondary rounded-lg border border-border">
            {/* Number Input */}
            <input
              type="text"
              inputMode="numeric"
              value={betAmountInput}
              onChange={(e) => {
                // Allow only numbers
                const value = e.target.value.replace(/[^0-9]/g, '');
                setBetAmountInput(value);
                // Update betAmount if valid
                const num = parseInt(value) || 0;
                if (num > 0) {
                  setBetAmount(num);
                }
              }}
              onBlur={() => {
                // Validate on blur - ensure minimum of 1 and max of channel points
                const num = parseInt(betAmountInput) || 1;
                const maxPoints = channelPoints || 250000;
                const clamped = Math.min(Math.max(1, num), maxPoints);
                setBetAmount(clamped);
                setBetAmountInput(clamped.toString());
              }}
              className="w-24 px-2 py-1.5 glass-input text-textPrimary text-sm font-medium focus:outline-none"
              placeholder="Amount"
            />
            
            {/* Quick Amount Buttons - fewer presets */}
            <div className="flex gap-1">
              {[10, 100, 1000].map(amount => (
                <button
                  key={amount}
                  onClick={() => {
                    setBetAmount(amount);
                    setBetAmountInput(amount.toString());
                  }}
                  className={`px-2 py-1.5 text-xs font-medium rounded transition-colors border ${
                    betAmount === amount 
                      ? 'bg-accent/30 border-accent/60 text-accent'
                      : 'bg-background border-border text-textSecondary hover:bg-backgroundSecondary'
                  }`}
                >
                  {amount >= 1000 ? `${amount / 1000}k` : amount}
                </button>
              ))}
              {channelPoints && (
                <button
                  onClick={() => {
                    setBetAmount(channelPoints);
                    setBetAmountInput(channelPoints.toString());
                  }}
                  className="px-2 py-1.5 text-xs font-bold bg-accent/30 hover:bg-accent/40 border border-accent/60 rounded transition-colors text-accent"
                >
                  ALL
                </button>
              )}
            </div>
            
            {/* Bet Button - compact */}
            <button
              onClick={handlePlacePrediction}
              disabled={!selectedOutcome || isSubmitting}
              className={`px-3 py-1.5 rounded text-xs font-bold transition-all whitespace-nowrap ${
                selectedOutcome && !isSubmitting
                  ? 'bg-accent hover:bg-accent-hover text-white'
                  : 'bg-background border border-border text-textSecondary cursor-not-allowed'
              }`}
            >
              {isSubmitting ? '...' : selfStake ? 'Add' : 'Bet'}
            </button>
          </div>
        </div>
      )}

      {/* Status indicators */}
      {selfStake && !isLocked && resolutionState === 'none' && (
        <div className="px-3 pb-3">
          <div className="py-2 px-3 bg-success/20 border border-success/50 rounded-lg flex items-center justify-center gap-2">
            <CheckCircle2 className="w-4 h-4 text-success" />
            <span className="text-success text-sm font-semibold">
              {selfStake.points.toLocaleString()} on &ldquo;
              {activePrediction.outcomes.find(o => o.id === selfStake.outcomeId)?.title ?? 'your pick'}
              &rdquo;
            </span>
          </div>
        </div>
      )}
      
      {/* Resolution States - Win/Loss/Refund/Pending */}
      {resolutionState === 'pending' && (
        <div className="px-3 pb-3">
          <div className="py-3 px-4 bg-accent/20 border border-accent/50 rounded-lg flex items-center justify-center gap-2 animate-pulse">
            <div className="w-5 h-5 border-2 border-purple-400 border-t-transparent rounded-full animate-spin" />
            <span className="text-accent text-sm font-semibold">
              Resolving Prediction...
            </span>
          </div>
        </div>
      )}
      
      {resolutionState === 'win' && (
        <div className="px-3 pb-3">
          <div className="py-4 px-4 bg-gradient-to-r from-success/30 to-highlight-green/30 border border-success rounded-lg text-center animate-pulse">
            <div className="flex justify-center mb-2">
              <PartyPopper className="w-8 h-8 text-success" />
            </div>
            <span className="text-success text-lg font-bold">YOU WON!</span>
            {winningOutcomeId && (
              <p className="text-success/80 text-sm mt-1">
                <EmoteText text={activePrediction.outcomes.find(o => o.id === winningOutcomeId)?.title ?? ''} emoteMap={emoteMap} keyPrefix="pred-win" />
              </p>
            )}
          </div>
        </div>
      )}
      
      {resolutionState === 'loss' && (
        <div className="px-3 pb-3">
          <div className="py-4 px-4 bg-gradient-to-r from-error/30 to-highlight-red/30 border border-error rounded-lg text-center">
            <div className="flex justify-center mb-2">
              <XCircle className="w-8 h-8 text-error" />
            </div>
            <span className="text-error text-lg font-bold">Better Luck Next Time</span>
            {winningOutcomeId && (
              <p className="text-error/80 text-sm mt-1">
                Winner: <EmoteText text={activePrediction.outcomes.find(o => o.id === winningOutcomeId)?.title ?? ''} emoteMap={emoteMap} keyPrefix="pred-loss" />
              </p>
            )}
          </div>
        </div>
      )}
      
      {resolutionState === 'refund' && (
        <div className="px-3 pb-3">
          <div className="py-4 px-4 bg-gradient-to-r from-info/30 to-highlight-cyan/30 border border-info rounded-lg text-center">
            <div className="flex justify-center mb-2">
              <RefreshCw className="w-8 h-8 text-info" />
            </div>
            <span className="text-info text-lg font-bold">Points Refunded</span>
            <p className="text-info/80 text-sm mt-1">Prediction was cancelled</p>
          </div>
        </div>
      )}
      
      {resolutionState === 'announced' && (
        <div className="px-3 pb-3">
          <div className="py-4 px-4 bg-gradient-to-r from-accent/30 to-highlight-purple/30 border border-accent rounded-lg text-center">
            <div className="flex justify-center mb-2">
              <Trophy className="w-8 h-8 text-accent" />
            </div>
            <span className="text-accent text-lg font-bold">Prediction Ended</span>
            {winningOutcomeId && (
              <p className="text-accent/80 text-sm mt-1">
                Winner: <EmoteText text={activePrediction.outcomes.find(o => o.id === winningOutcomeId)?.title ?? ''} emoteMap={emoteMap} keyPrefix="pred-announced" />
              </p>
            )}
          </div>
        </div>
      )}
      
      {/* Normal Locked State (waiting for results) */}
      {isLocked && resolutionState === 'none' && (
        <div className="px-3 pb-3">
          <div className="py-2 px-3 bg-warning/20 border border-warning/50 rounded-lg flex items-center justify-center gap-2">
            <Hourglass className="w-4 h-4 text-warning animate-pulse" />
            <span className="text-warning text-sm font-semibold">
              Awaiting Results{selfStake && ` • ${selfStake.points.toLocaleString()} in`}
            </span>
          </div>
          {/* Total stats when locked */}
          <div className="mt-2 flex items-center justify-center gap-4 text-xs text-textSecondary">
            <span className="flex items-center gap-1">
              <Users className="w-3 h-3" />
              {activePrediction.outcomes.reduce((sum, o) => sum + o.total_users, 0).toLocaleString()} voters
            </span>
            <span className="flex items-center gap-1">
              {customPointsIconUrl ? (
                <img src={customPointsIconUrl} alt="points" className="w-3 h-3" />
              ) : (
                <ChannelPointsIcon size={12} className="text-textSecondary" />
              )}
              {activePrediction.outcomes.reduce((sum, o) => sum + o.total_points, 0).toLocaleString()} points
            </span>
          </div>
        </div>
      )}
    </OverlayBanner>
  );
};

export default PredictionOverlay;
