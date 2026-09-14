// Channel picker for a Drops campaign. Surfaces WHERE a reward can be earned:
//  - ACL campaigns (allow-listed channels only) list exactly those channels with
//    their live status, so the user knows if any are live to earn on right now.
//  - Open campaigns list the live channels currently streaming the game.
// Picking a live channel hands it back via onPick; the caller decides what that
// means (core opens the player to watch; the automation plugin collects it). The whole
// point is feedback: an ACL campaign with nothing live says so instead of doing
// nothing.

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X, Radio, Loader2, Users } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { Logger } from '../../utils/logger';
import type { AllowedChannel, TwitchStream, UserInfo } from '../../types';

export interface PickableChannel {
  login: string;
  displayName: string;
  userId: string;
  viewerCount: number;
  isLive: boolean;
  /** The category the channel is currently streaming (live only). This IS an
   *  eligibility gate: an allow-listed channel only earns the drop while live
   *  under the campaign's own category. */
  currentGame?: string;
  currentGameId?: string;
  avatarUrl?: string;
  /** The live stream object (carries game_name etc.) so the caller can hand it
   *  straight to startStream — same as clicking a stream card, which is what makes
   *  the drop-progress badge light up. */
  stream?: TwitchStream;
}

interface ChannelPickerModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** The campaign's id, used to ask Twitch which of the allow-listed channels
   *  are actually offering the drop right now. */
  campaignId: string;
  campaignName: string;
  gameName: string;
  /** The campaign's category id. Usually a drop only credits while the channel
   *  is live under this category, so it is the fallback filter for ACL picks
   *  when the eligibility lookup can't be reached. */
  gameId?: string;
  allowedChannels: AllowedChannel[];
  isAclBased: boolean;
  /** Verb for the action, e.g. "Watch" (core) or "Collect" (automation plugin). */
  actionLabel?: string;
  onPick: (channel: PickableChannel) => void;
}

function formatViewers(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
  return String(count);
}

function streamToChannel(s: TwitchStream): PickableChannel {
  return {
    login: s.user_login,
    displayName: s.user_name || s.user_login,
    userId: s.user_id,
    viewerCount: s.viewer_count || 0,
    isLive: true,
    currentGame: s.game_name,
    currentGameId: s.game_id,
    avatarUrl: s.profile_image_url,
    stream: s,
  };
}

export default function ChannelPickerModal({
  isOpen,
  onClose,
  campaignId,
  campaignName,
  gameName,
  gameId,
  allowedChannels,
  isAclBased,
  actionLabel = 'Watch',
  onPick,
}: ChannelPickerModalProps) {
  const [channels, setChannels] = useState<PickableChannel[]>([]);
  const [eligibleIds, setEligibleIds] = useState<Set<string> | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    setEligibleIds(null);
    try {
      let result: PickableChannel[];
      if (isAclBased && allowedChannels.length > 0) {
        // Allow-listed campaign: only the permitted channels that are LIVE right now
        // (offline ones can't be watched/collected, so we don't list them at all).
        // One batched call (chunked 100/request in Rust) instead of one request per
        // channel — a special-event ACL like EWC has ~1180 channels, and the old
        // per-channel fan-out rate-limited and silently dropped live ones.
        const logins = allowedChannels.map(ch => ch.name);
        const streams = await invoke<TwitchStream[]>('check_streams_online', { userLogins: logins });
        result = (streams || []).map(streamToChannel);

        // Which of those are actually earning this drop right now, per Twitch.
        // Failing this lookup leaves the list on the category fallback below
        // rather than emptying the picker.
        try {
          const eligible = await invoke<string[]>('get_campaign_eligible_channels', {
            campaignId,
            channelIds: result.map(c => c.userId),
          });
          setEligibleIds(new Set(eligible || []));
        } catch (err) {
          Logger.warn('[ChannelPicker] eligibility lookup failed, using category match:', err);
        }
      } else {
        // Open campaign: list whoever is live in the game right now.
        const [streams] = await invoke<[TwitchStream[], string | null]>('get_streams_by_game_name', {
          gameName,
          excludeUserLogin: null,
          cursor: null,
          limit: 40,
        });
        result = (streams || []).map(streamToChannel);
      }
      // Most viewers first.
      result.sort((a, b) => b.viewerCount - a.viewerCount);
      setChannels(result);

      // The stream/live endpoints don't carry avatars, so fill them in from the
      // user lookup after the list is already on screen (progressive, not blocking).
      const needAvatars = result.filter(c => !c.avatarUrl);
      if (needAvatars.length > 0) {
        const found = await Promise.all(
          needAvatars.map(async (c) => {
            try {
              const u = await invoke<UserInfo>('get_user_by_login', { login: c.login });
              return [c.login, u.profile_image_url] as const;
            } catch {
              return [c.login, undefined] as const;
            }
          })
        );
        const byLogin = new Map(found);
        setChannels(prev => prev.map(c => (c.avatarUrl || !byLogin.get(c.login)) ? c : { ...c, avatarUrl: byLogin.get(c.login) }));
      }
    } catch (err) {
      Logger.error('[ChannelPicker] failed to load channels:', err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsLoading(false);
    }
  }, [isAclBased, allowedChannels, gameName, campaignId]);

  useEffect(() => {
    if (isOpen) load();
  }, [isOpen, load]);

  if (!isOpen) return null;

  // Live channels that can earn this drop right now.
  //  - Open campaigns already fetched only channels live in the campaign's game.
  //  - ACL campaigns: the allow-list is the FIRST gate, and being on it is not
  //    always enough — an allow-listed streamer live in some other category
  //    usually earns nothing for THIS drop, so listing the whole live roster
  //    offered channels that can't credit. But "same category" is not the rule
  //    either: a creator campaign like Ironmouse's subathon is filed under
  //    Special Events and credits on her channel whatever she is playing, so
  //    the category test hid the ONLY channel that could earn it. Twitch answers
  //    this per channel (`viewerDropCampaigns`), so that answer decides.
  //  - Category match is the fallback for when that lookup fails, since a stale
  //    heuristic beats an empty picker.
  const isEarnable = (c: PickableChannel): boolean => {
    if (!isAclBased) return true; // open campaign list is already category-correct
    if (eligibleIds) return eligibleIds.has(c.userId);
    if (gameId && c.currentGameId) return c.currentGameId === gameId;
    return !!c.currentGame && !!gameName && c.currentGame.toLowerCase() === gameName.toLowerCase();
  };
  const earnable = channels.filter(c => c.isLive && isEarnable(c));

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center p-4 sm:p-6">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm animate-in fade-in duration-200" onClick={onClose} />

      <div className="relative w-full max-w-md max-h-[80vh] bg-background rounded-xl shadow-2xl border border-borderLight overflow-hidden flex flex-col animate-in fade-in zoom-in-95 duration-200">
        {/* Header */}
        <div className="flex items-center gap-3 px-5 py-4 border-b border-borderLight bg-backgroundSecondary">
          <div className="flex-1 min-w-0">
            <h3 className="font-bold text-textPrimary text-base truncate">Choose a channel</h3>
            <p className="text-xs text-textSecondary mt-0.5 truncate">
              {campaignName} · {gameName}
            </p>
          </div>
          <button onClick={onClose} className="p-2 text-textSecondary hover:text-textPrimary hover:bg-surface rounded-lg transition-all">
            <X size={18} />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto custom-scrollbar p-4">
          {isLoading ? (
            <div className="flex flex-col items-center justify-center py-12 text-textSecondary">
              <Loader2 className="w-7 h-7 animate-spin text-accent mb-3" />
              <span className="text-sm">Checking who's live...</span>
            </div>
          ) : error ? (
            <div className="flex flex-col items-center justify-center py-10 text-center">
              <p className="text-sm text-textSecondary mb-3">Couldn't load channels.</p>
              <button onClick={load} className="glass-button px-3 py-1.5 text-xs font-semibold text-accent">Retry</button>
            </div>
          ) : earnable.length === 0 ? (
            <div className="py-8 text-center">
              <Radio className="w-10 h-10 text-textMuted opacity-40 mx-auto mb-3" />
              <p className="text-sm font-medium text-textPrimary mb-1">Nothing to earn this on right now</p>
              <p className="text-xs text-textSecondary max-w-[18rem] mx-auto leading-snug">
                {isAclBased
                  ? `This reward only drops on its participating channels while they're live. None are right now, so check back later.`
                  : `No one is streaming ${gameName} right now. Check back later.`}
              </p>
            </div>
          ) : (
            <div className="space-y-1.5">
              {earnable.map(c => (
                <button
                  key={c.login}
                  onClick={() => onPick(c)}
                  className="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg bg-backgroundSecondary hover:bg-surface border border-transparent hover:border-borderLight transition-all text-left"
                >
                  <ChannelAvatar channel={c} />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium text-textPrimary truncate block">{c.displayName}</span>
                    <span className="flex items-center gap-1.5 text-[11px] text-textSecondary mt-0.5">
                      <span className="flex items-center gap-1 text-red-400 font-semibold">
                        <span className="relative flex h-1.5 w-1.5">
                          <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-red-400 opacity-75" />
                          <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-red-500" />
                        </span>
                        LIVE
                      </span>
                      <span className="flex items-center gap-1"><Users size={11} />{formatViewers(c.viewerCount)}</span>
                    </span>
                  </div>
                  <span className="glass-button px-2.5 py-1 text-xs font-semibold text-accent shrink-0">{actionLabel}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ChannelAvatar({ channel }: { channel: PickableChannel }) {
  if (channel.avatarUrl) {
    return (
      <Tooltip content={channel.displayName} side="top" delay={300}>
        <img src={channel.avatarUrl} alt={channel.displayName} className="w-8 h-8 rounded-full object-cover border border-borderLight shrink-0" loading="lazy" />
      </Tooltip>
    );
  }
  return (
    <div className="w-8 h-8 rounded-full bg-surface border border-borderLight flex items-center justify-center text-xs font-semibold text-textSecondary shrink-0">
      {channel.displayName.charAt(0).toUpperCase()}
    </div>
  );
}
