// MultiChatPane — channel-keyed chat surface used inside the StreamNook
// MultiChat popout window.
//
// Mounts the main app's ChatWidget with a richer `channelOverride` so the
// popout reaches feature parity with the in-app chat: copy, reply, pinned
// messages, emote picker, mod menu, profile clicks, badge interactions —
// AND the stream-view chrome (viewer count, uptime, About panel, etc.) reads
// real values because this pane polls Helix for the live stream metadata.
//
// Polling cadence: check_stream_online every 30s. Cheaper than EventSub and
// good enough for the popout's at-a-glance "what's happening" use case.
// Hype train / raid events still require EventSub (which is single-broadcaster
// today); those remain main-app-only until EventSub is made multi-broadcaster.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ChatWidget, { type ChatWidgetChannelOverride } from '../ChatWidget';
import type { TwitchStream, HypeTrainData } from '../../types';
import type { ProviderId } from '../../types/providers';
import { Logger } from '../../utils/logger';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { useActivityStore } from '../../stores/activityStore';
import { ensureChannelHistory } from '../../stores/chatConnectionStore';
import { makeKey } from '../../utils/providerKey';

export interface MultiChatPaneProps {
  channel: string;
  channelId?: string | null;
  channelName?: string;
  /** Source platform. Absent/twitch uses the full ChatWidget; other providers
   *  use the lightweight read-only ProviderChatPane. */
  provider?: ProviderId;
  /** Whether this pane is the active/focused tab — it owns the keyboard-mod keys. */
  isActive?: boolean;
  /** Saved message filter bound to this pane (persisted per tab by the window). */
  filterId?: string | null;
  onFilterIdChange?: (id: string | null) => void;
}

const STREAM_POLL_INTERVAL_MS = 30_000;

interface ChannelUserInfo {
  id?: string;
  login?: string;
  display_name?: string;
  profile_image_url?: string;
  broadcaster_type?: string;
}

function TwitchChatPane({ channel, channelId, channelName, isActive, filterId, onFilterIdChange }: MultiChatPaneProps) {
  const channelKey = channel.toLowerCase();

  const [stream, setStream] = useState<TwitchStream | null>(null);
  const [userInfo, setUserInfo] = useState<ChannelUserInfo | null>(null);
  // Consecutive null `check_stream_online` results — debounces transient poll
  // glitches so a live stream's uptime doesn't flicker (see fetchStream).
  const offlineStreakRef = useRef(0);

  // Resolve channel-level metadata (display name, avatar, broadcaster type)
  // once per channel. Doesn't change between live/offline.
  useEffect(() => {
    let active = true;
    (async () => {
      try {
        const info = await invoke<ChannelUserInfo>('get_user_by_login', { login: channelKey });
        if (!active) return;
        setUserInfo(info);
        // If this channel was opened WITHOUT a resolved id (a Go Live seed or a
        // saved source stored without one), the acquire-time history backfill
        // bailed. Now that we have the id, pull recent chat so an OFFLINE channel
        // shows its messages instead of an empty pane (core app does this too).
        if (!channelId && info?.id) void ensureChannelHistory(channelKey, info.id);
      } catch (err) {
        Logger.warn('[MultiChatPane] get_user_by_login failed:', err);
      }
    })();
    return () => {
      active = false;
    };
  }, [channelKey, channelId]);

  // Poll live stream metadata so viewer count, uptime, title, and game stay
  // current. check_stream_online returns the full TwitchStream when online
  // and null when offline. Visibility-gated so a hidden window stops polling.
  const fetchStream = useCallback(async () => {
    try {
      const s = await invoke<TwitchStream | null>('check_stream_online', {
        userLogin: channelKey,
      });
      if (s) {
        offlineStreakRef.current = 0;
        setStream(s);
      } else {
        // A single null is often a transient poll glitch (several panes poll at
        // once), so don't drop a live stream's uptime on one miss — require two
        // consecutive nulls before treating it as genuinely offline. Keeps the
        // uptime ticking steadily instead of flickering.
        offlineStreakRef.current += 1;
        if (offlineStreakRef.current >= 2) setStream(null);
      }
    } catch (err) {
      Logger.warn('[MultiChatPane] check_stream_online failed:', err);
    }
  }, [channelKey]);
  // Initial fetch on mount / channel change. Inline async (setState only AFTER the
  // await, inside a callback) so it doesn't trip the cascading-render guard.
  useEffect(() => {
    let active = true;
    offlineStreakRef.current = 0; // fresh channel
    void (async () => {
      try {
        const s = await invoke<TwitchStream | null>('check_stream_online', {
          userLogin: channelKey,
        });
        if (!active) return;
        if (s) {
          offlineStreakRef.current = 0;
          setStream(s);
        } else {
          offlineStreakRef.current += 1;
          if (offlineStreakRef.current >= 2) setStream(null);
        }
      } catch (err) {
        Logger.warn('[MultiChatPane] check_stream_online failed:', err);
      }
    })();
    return () => {
      active = false;
    };
  }, [channelKey]);
  useVisibleInterval(fetchStream, STREAM_POLL_INTERVAL_MS);

  const channelOverride = useMemo<ChatWidgetChannelOverride>(() => {
    const liveUserId = stream?.user_id || channelId || userInfo?.id || '';
    const liveLogin = stream?.user_login || channelKey;
    const liveName =
      stream?.user_name ||
      userInfo?.display_name ||
      channelName ||
      channelKey;

    return {
      provider: 'twitch',
      user_login: liveLogin,
      user_id: liveUserId,
      user_name: liveName,
      title: stream?.title,
      game_name: stream?.game_name,
      viewer_count: stream?.viewer_count,
      started_at: stream?.started_at,
      thumbnail_url: stream?.thumbnail_url,
      profile_image_url: userInfo?.profile_image_url ?? stream?.profile_image_url,
      broadcaster_type: userInfo?.broadcaster_type ?? stream?.broadcaster_type,
      is_live: stream !== null,
      is_active: isActive,
    };
  }, [stream, userInfo, channelKey, channelId, channelName, isActive]);

  // Hype Train: poll this channel's train (auth-free GQL, works for any channel,
  // unaffected by the Jan-2026 EventSub v1 withdrawal) and surface its start +
  // each level-up in the combined activity panel, Golden Kappa flagged. Adaptive:
  // 3s while a train runs, 15s idle. Only polls while the channel is live, since
  // trains only run on live channels. The per-train+level event id dedups, so a
  // poller restart (e.g. on go-live) never double-posts a level already seen.
  const hypeChannelId = stream?.user_id || channelId || userInfo?.id || '';
  const isLive = stream !== null;
  // Per-pane hype train: drives both the in-pane progress banner (passed to
  // ChatWidget) and the combined activity-panel start/level-up rows.
  const [paneHypeTrain, setPaneHypeTrain] = useState<HypeTrainData | null>(null);
  // Clear any stale hype train the moment the channel goes offline. Adjusted during
  // render (React's supported alternative to a setState-in-effect for syncing state
  // to a value), not in an effect.
  if (!isLive && paneHypeTrain !== null) setPaneHypeTrain(null);
  useEffect(() => {
    if (!hypeChannelId || !isLive) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let prevLevel = 0;
    let wasActive = false;
    const poll = async () => {
      if (cancelled) return;
      let active = false;
      let imminent = false;
      try {
        const s = await invoke<{
          is_active: boolean;
          id?: string;
          level: number;
          progress: number;
          goal: number;
          total: number;
          started_at?: string;
          expires_at?: string;
          is_golden_kappa: boolean;
        }>('get_hype_train_status', { channelId: hypeChannelId, channelLogin: channelKey });
        active = s.is_active;
        // Poll fast (1s) when a level-up is imminent so the celebration fires ASAP.
        imminent = s.is_active && s.goal > 0 && s.progress / s.goal > 0.85;
        if (s.is_active && s.level >= 1) {
          // Feed the in-pane banner with the full live status every poll.
          setPaneHypeTrain({
            id: s.id || '',
            broadcaster_user_id: hypeChannelId,
            broadcaster_user_login: channelKey,
            broadcaster_user_name: channelName || channelKey,
            level: s.level,
            total: s.total,
            progress: s.progress,
            goal: s.goal,
            top_contributions: [],
            started_at: s.started_at || '',
            expires_at: s.expires_at || '',
            is_golden_kappa: s.is_golden_kappa,
          });
          // Level 1 = the train starting; higher levels = level-ups.
          if (s.level > prevLevel || !wasActive) {
            prevLevel = s.level;
            useActivityStore.getState().addEvent({
              id: `hype-${channelKey}-${s.id ?? 'train'}-L${s.level}`,
              timestamp: new Date().toISOString(),
              provider: 'twitch',
              channel: makeKey('twitch', channelKey),
              channel_display: channelName || channelKey,
              kind: 'hypetrain',
              actor: { username: channelKey, display_name: channelName || channelKey },
              system_text: s.is_golden_kappa ? `Level ${s.level} · Golden Kappa` : `Level ${s.level}`,
            });
          }
          wasActive = true;
        } else {
          setPaneHypeTrain(null);
          wasActive = false;
          prevLevel = 0;
        }
      } catch (err) {
        Logger.warn('[MultiChatPane] get_hype_train_status failed:', err);
      }
      if (!cancelled) timer = setTimeout(poll, active ? (imminent ? 1000 : 3000) : 15000);
    };
    timer = setTimeout(poll, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [hypeChannelId, isLive, channelKey, channelName]);

  return (
    <ChatWidget
      channelOverride={channelOverride}
      hypeTrainOverride={paneHypeTrain}
      filterId={filterId}
      onFilterIdChange={onFilterIdChange}
    />
  );
}

// Live channel metadata captured by the backend during channel resolve. Shared by
// Kick (user_id is the numeric broadcaster id) and YouTube (user_id is the UC…
// channel id string) — the JSON field names match, so one shape reads both.
interface ProviderChannelMeta {
  user_id?: number | string | null;
  username?: string | null; // properly-cased display name
  viewer_count?: number | null;
  start_time?: string | null; // ISO-UTC
  title?: string | null;
  profile_pic?: string | null;
  is_live: boolean;
}

// The backend command that returns a provider's channel metadata, or null if the
// provider exposes none.
function metaCommandFor(provider?: ProviderId): string | null {
  if (provider === 'kick') return 'get_kick_channel_meta';
  if (provider === 'youtube') return 'get_youtube_channel_meta';
  if (provider === 'tiktok') return 'get_tiktok_channel_meta';
  return null;
}

// Non-Twitch sources render through the SAME ChatWidget for full parity (emotes,
// picker, replies, badges). ChatWidget reads the already-connected
// `provider:channel` slice and gates off every Twitch-only behavior on `provider`.
// The chrome (viewers / uptime / title / avatar) is Kick-driven: we read the
// metadata the backend captured from the Kick channel API at resolve time. Uptime
// ticks live in ChatWidget from `started_at`; viewer count is the resolve-time
// snapshot until a live refresh lands (the Kick API is Cloudflare-gated).
function ProviderViaChatWidget({ channel, channelId, channelName, provider, isActive }: MultiChatPaneProps) {
  const slug = channel.toLowerCase();
  const [meta, setMeta] = useState<ProviderChannelMeta | null>(null);
  const metaCommand = metaCommandFor(provider);

  const fetchMeta = useCallback(async () => {
    if (!metaCommand) return; // provider exposes no channel metadata
    try {
      const m = await invoke<ProviderChannelMeta | null>(metaCommand, { slug });
      if (m) setMeta(m);
    } catch (err) {
      Logger.warn(`[MultiChatPane] ${metaCommand} failed:`, err);
    }
  }, [slug, metaCommand]);
  // The pane mounts before the (multi-second) resolve caches the meta — Kick clears
  // Cloudflare in a hidden webview, YouTube fetches + parses the watch page — so
  // poll FAST until it lands, otherwise the name / viewers / uptime wouldn't appear
  // until the next slow-poll tick (up to 30s). Stops the instant meta is acquired;
  // capped so a never-resolving channel can't fast-poll forever. The slow interval
  // below then covers any later refresh.
  useEffect(() => {
    if (!metaCommand || meta) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let attempts = 0;
    const tick = async () => {
      if (cancelled) return;
      const m = await invoke<ProviderChannelMeta | null>(metaCommand, { slug }).catch(() => null);
      if (cancelled) return;
      if (m) {
        setMeta(m);
        return;
      }
      if (++attempts < 50) timer = setTimeout(tick, 500);
    };
    timer = setTimeout(tick, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [metaCommand, slug, meta]);
  useVisibleInterval(fetchMeta, STREAM_POLL_INTERVAL_MS);

  // Normalize the tab/source label to the resolved display name (e.g. a YouTube
  // "@jynxzi" the user typed becomes "Jynxzi"). Fires once the channel metadata
  // lands; MultiChatWindow's listener updates the entry's channelName.
  useEffect(() => {
    const name = meta?.username;
    if (provider && name) {
      window.dispatchEvent(
        new CustomEvent('multichat-source-resolved', {
          detail: { provider, channel, displayName: name },
        }),
      );
    }
  }, [meta?.username, provider, channel]);

  const channelOverride = useMemo<ChatWidgetChannelOverride>(
    () => ({
      provider,
      user_login: slug,
      user_id: meta?.user_id != null ? String(meta.user_id) : (channelId ?? ''),
      user_name: meta?.username || channelName || channel,
      title: meta?.title ?? undefined,
      viewer_count: meta?.viewer_count ?? undefined,
      started_at: meta?.start_time ?? undefined,
      profile_image_url: meta?.profile_pic ?? undefined,
      is_live: meta?.is_live ?? true,
      is_active: isActive,
    }),
    [provider, slug, meta, channelId, channelName, channel, isActive],
  );
  return <ChatWidget channelOverride={channelOverride} />;
}

// This wrapper holds no hooks before the branch (rules-of-hooks).
export function MultiChatPane(props: MultiChatPaneProps) {
  if (props.provider && props.provider !== 'twitch') {
    return <ProviderViaChatWidget {...props} />;
  }
  return <TwitchChatPane {...props} />;
}

export default MultiChatPane;
