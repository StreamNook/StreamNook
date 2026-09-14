import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../services/helix';
import { Trophy, TrendingUp, ExternalLink, RefreshCw } from 'lucide-react';
import { ChannelPointsBalance } from '../types';
import { useAppStore } from '../stores/AppStore';

import { Logger } from '../utils/logger';
import { Tooltip } from './ui/Tooltip';
interface ChannelPointsLeaderboardProps {
  onStreamClick?: (channelName: string) => void;
  // Reports the all-followed-channel totals whenever balances update, so a
  // parent (e.g. the Drops stats cards) can show the account-wide figure.
  onTotalsChange?: (totalPoints: number, channelCount: number) => void;
}

const ChannelPointsLeaderboard = ({ onStreamClick, onTotalsChange }: ChannelPointsLeaderboardProps) => {
  const [balances, setBalances] = useState<ChannelPointsBalance[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [profilePics, setProfilePics] = useState<Record<string, string>>({});
  // Action without a subscription; only the followed-streams list (read for
  // live status and click-through) drives re-renders.
  const { startStream } = useAppStore.getState();
  const followedStreams = useAppStore((s) => s.followedStreams);

  useEffect(() => {
    // On open, pull every followed channel's balance (throttled backend-side);
    // the interval then just re-reads the warm store, which is cheap.
    refreshAll(false);
    const interval = setInterval(fetchBalances, 30000);

    return () => clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const applyBalances = async (data: ChannelPointsBalance[]) => {
    // Sort by balance descending (highest first)
    const sorted = [...data].sort((a, b) => b.balance - a.balance);
    setBalances(sorted);
    onTotalsChange?.(sorted.reduce((sum, b) => sum + b.balance, 0), sorted.length);
    // Fetch profile pictures for all users
    await fetchProfilePictures(sorted);
  };

  const fetchBalances = async () => {
    try {
      const data = await invoke<ChannelPointsBalance[]>('get_all_channel_points_balances');
      await applyBalances(data);
    } catch (error) {
      Logger.error('Failed to fetch channel points balances:', error);
    } finally {
      setLoading(false);
    }
  };

  // Query every channel you follow (live or not) and refill the store, so the
  // board reflects your whole account, not just channels watched this session.
  const refreshAll = async (force: boolean) => {
    setRefreshing(true);
    try {
      const data = await invoke<ChannelPointsBalance[]>('refresh_followed_channel_points', { force });
      await applyBalances(data);
    } catch (error) {
      Logger.error('Failed to refresh followed channel points:', error);
      await fetchBalances();
    } finally {
      setRefreshing(false);
      setLoading(false);
    }
  };

  const fetchProfilePictures = async (balanceData: ChannelPointsBalance[]) => {
    try {

      // Twitch's /users endpoint caps at 100 logins per request. The list now
      // spans every followed channel, so chunk it: one oversized request 400s
      // and drops every picture. Dedupe and skip blanks too.
      const logins = Array.from(
        new Set(
          balanceData
            .map((b) => b.channel_name?.toLowerCase())
            .filter((name): name is string => !!name),
        ),
      );

      const pics: Record<string, string> = {};
      for (let i = 0; i < logins.length; i += 100) {
        const queryParams = logins
          .slice(i, i + 100)
          .map((name) => `login=${encodeURIComponent(name)}`)
          .join('&');

        let data: { data?: Array<{ profile_image_url?: string; login?: string }> };
        try {
          data = await helixGet('users', queryParams);
        } catch (e) {
          // Keep the pictures the other chunks resolved rather than losing all.
          Logger.error('Failed to fetch profile pictures chunk:', e);
          continue;
        }
        if (data.data && Array.isArray(data.data)) {
          data.data.forEach((user: { profile_image_url?: string; login?: string }) => {
            if (user.profile_image_url && user.login) {
              pics[user.login.toLowerCase()] = user.profile_image_url;
            }
          });
        }
      }

      setProfilePics(pics);
    } catch (error) {
      Logger.error('Failed to fetch profile pictures:', error);
    }
  };

  const handleStreamClick = (channelName: string) => {
    if (onStreamClick) {
      onStreamClick(channelName);
    } else {
      // Find stream info from followed streams
      const streamInfo = followedStreams.find(
        s => s.user_login.toLowerCase() === channelName.toLowerCase()
      );

      if (streamInfo) {
        startStream(channelName, streamInfo);
      } else {
        startStream(channelName);
      }
    }
  };

  const getStreamerInfo = (channelName: string) => {
    // Find the stream in followed streams to get live status
    const stream = followedStreams.find(
      s => s.user_login.toLowerCase() === channelName.toLowerCase()
    );

    return {
      profilePicUrl: stream?.profile_image_url || null,
      isLive: !!stream,
      stream: stream || null
    };
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <div className="w-6 h-6 border-2 border-accent border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  if (balances.length === 0) {
    return (
      <div className="text-center py-8">
        <TrendingUp className="w-12 h-12 text-textSecondary mx-auto mb-3 opacity-50" />
        <p className="text-textSecondary text-sm">No channel points data yet</p>
        <p className="text-textTertiary text-xs mt-1">
          Watch streams to start earning points
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 mb-3">
        <Trophy className="w-4 h-4 text-accent" />
        <h3 className="text-textPrimary font-semibold text-sm">Channel Points Leaderboard</h3>
        <Tooltip content="Refresh every followed channel" side="top">
          <button
            onClick={() => refreshAll(true)}
            disabled={refreshing}
            className="ml-auto p-1 text-textSecondary hover:text-accent transition-colors disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin' : ''}`} />
          </button>
        </Tooltip>
      </div>

      <div className="space-y-1.5 max-h-96 overflow-y-auto pr-1 custom-scrollbar">
        {balances.map((balance, index) => {
          const streamerInfo = getStreamerInfo(balance.channel_name);

          // Define styles for top 3 positions
          const getCardStyle = () => {
            if (index === 0) {
              return 'relative overflow-hidden bg-gradient-to-r from-yellow-500/10 via-yellow-400/5 to-yellow-500/10 border-yellow-500/30';
            } else if (index === 1) {
              return 'relative overflow-hidden bg-gradient-to-r from-gray-400/10 via-gray-300/5 to-gray-400/10 border-gray-400/30';
            } else if (index === 2) {
              return 'relative overflow-hidden bg-gradient-to-r from-orange-500/10 via-orange-400/5 to-orange-500/10 border-orange-500/30';
            }
            return '';
          };

          const getRankBadgeStyle = () => {
            if (index === 0) return 'bg-yellow-500/20 text-yellow-400 border-yellow-500/40';
            if (index === 1) return 'bg-gray-400/20 text-gray-300 border-gray-400/40';
            if (index === 2) return 'bg-orange-600/20 text-orange-400 border-orange-500/40';
            return 'bg-glass text-textSecondary border-borderLight';
          };

          return (
            <div
              key={balance.channel_id}
              className={`flex items-center gap-3 p-2.5 glass-panel rounded-lg hover:bg-glass transition-all group border ${getCardStyle()}`}
            >
              {/* Shimmer effect for top 3 */}
              {index < 3 && (
                <div
                  className="absolute inset-0 opacity-30 pointer-events-none"
                  style={{
                    background: index === 0
                      ? 'linear-gradient(90deg, transparent 0%, transparent 40%, rgba(251, 191, 36, 0.2) 50%, transparent 60%, transparent 100%)'
                      : index === 1
                        ? 'linear-gradient(90deg, transparent 0%, transparent 40%, rgba(203, 213, 225, 0.2) 50%, transparent 60%, transparent 100%)'
                        : 'linear-gradient(90deg, transparent 0%, transparent 40%, rgba(249, 115, 22, 0.2) 50%, transparent 60%, transparent 100%)',
                    backgroundSize: '200% 100%',
                    animation: 'shimmer 3s ease-in-out infinite',
                  }}
                />
              )}

              {/* Rank Badge */}
              <div className={`flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center text-xs font-bold border ${getRankBadgeStyle()} relative z-10`}>
                {index + 1}
              </div>

              {/* Profile Picture */}
              <div className="relative flex-shrink-0 z-10">
                {profilePics[balance.channel_name.toLowerCase()] ? (
                  <img
                    src={profilePics[balance.channel_name.toLowerCase()]}
                    alt={balance.channel_name}
                    className={`w-9 h-9 rounded-full object-cover border ${streamerInfo.isLive ? 'border-green-500' : 'border-borderLight'}`}
                    onError={(e) => {
                      // Fallback to first letter if image fails
                      e.currentTarget.style.display = 'none';
                      const sibling = e.currentTarget.nextElementSibling;
                      if (sibling) sibling.classList.remove('hidden');
                    }}
                  />
                ) : null}
                <div
                  className={`${profilePics[balance.channel_name.toLowerCase()] ? 'hidden' : ''} w-9 h-9 rounded-full bg-accent/20 flex items-center justify-center text-accent font-bold text-sm`}
                >
                  {balance.channel_name.charAt(0).toUpperCase()}
                </div>
                {/* Live indicator dot */}
                {streamerInfo.isLive && (
                  <div className="absolute -bottom-0.5 -right-0.5 w-3 h-3 bg-green-500 rounded-full border-2 border-background" />
                )}
              </div>

              {/* Streamer Info */}
              <div className="flex-1 min-w-0">
                <Tooltip content={`Watch ${balance.channel_name}`} side="top">
                  <button
                    onClick={() => handleStreamClick(balance.channel_name)}
                    className="text-textPrimary text-sm font-medium hover:text-accent transition-colors truncate block w-full text-left group-hover:underline"
                  >
                    {balance.channel_name}
                    <ExternalLink className="inline-block w-3 h-3 ml-1 opacity-0 group-hover:opacity-100 transition-opacity" />
                  </button>
                </Tooltip>
                {streamerInfo.isLive && (
                  <span className="text-xs text-green-400">Live</span>
                )}
              </div>

              {/* Points Display */}
              <div className="flex-shrink-0 text-right">
                <div className="text-accent font-bold text-sm">
                  {balance.balance.toLocaleString()}
                </div>
                <div className="text-textTertiary text-xs">
                  pts
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Summary */}
      <div className="mt-3 pt-3 border-t border-borderSubtle">
        <div className="flex items-center justify-between text-xs">
          <span className="text-textSecondary">Total Streamers</span>
          <span className="text-textPrimary font-medium">{balances.length}</span>
        </div>
        <div className="flex items-center justify-between text-xs mt-1">
          <span className="text-textSecondary">Total Points</span>
          <span className="text-accent font-bold">
            {balances.reduce((sum, b) => sum + b.balance, 0).toLocaleString()}
          </span>
        </div>
      </div>
    </div>
  );
};

export default ChannelPointsLeaderboard;
