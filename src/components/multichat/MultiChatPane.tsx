// MultiChatPane — channel-keyed chat surface used inside the StreamNook
// MultiChat popout window.
//
// Mounts the main app's ChatWidget with a richer `channelOverride` so the
// popout reaches feature parity with the in-app chat: copy, reply, pinned
// messages, emote picker, mod menu, profile clicks, badge interactions —
// AND the stream-view chrome (viewer count, uptime, About panel, etc.) reads
// real values: a Twitch pane takes them from Rust's channel state (one Helix
// batch for every watched channel), a Kick or YouTube pane polls its
// platform's channel metadata every 30 s.
// Raid events still require EventSub (which is single-broadcaster today); the
// hype train comes from the shared Rust watch.

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ChatWidget, { type ChatWidgetChannelOverride } from '../ChatWidget';
import type { HypeTrainData } from '../../types';
import type { ProviderId } from '../../types/providers';
import { Logger } from '../../utils/logger';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { ensureChannelHistory } from '../../stores/chatConnectionStore';
import { useChannelState } from '../../stores/channelStateStore';
import { recordHypeTrainActivity, watchHypeTrains } from '../../services/hypeTrainWatch';

export interface MultiChatPaneProps {
  channel: string;
  channelId?: string | null;
  channelName?: string;
  /** Source platform. Absent/twitch uses the full ChatWidget; other providers
   *  go through ProviderViaChatWidget below, which wraps the same widget in
   *  read-only mode. */
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

  const [userInfo, setUserInfo] = useState<ChannelUserInfo | null>(null);
  // The live broadcast (viewers, start, title, category) comes from Rust's
  // channel state, which this pane's ChatWidget watches: one Helix batch for
  // every channel any window shows, a live channel only reading offline after
  // two missed polls. Until Rust answers, the channel reads offline, as it did
  // before the first poll here.
  const live = useChannelState(channelKey);
  const isLive = !!live?.started_at;

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

  const channelOverride = useMemo<ChatWidgetChannelOverride>(() => {
    const liveUserId = channelId || userInfo?.id || '';
    const liveName = userInfo?.display_name || channelName || channelKey;

    return {
      provider: 'twitch',
      user_login: channelKey,
      user_id: liveUserId,
      user_name: liveName,
      title: live?.title ?? undefined,
      game_name: live?.game_name ?? undefined,
      viewer_count: live?.viewer_count ?? undefined,
      started_at: live?.started_at ?? undefined,
      profile_image_url: userInfo?.profile_image_url,
      broadcaster_type: userInfo?.broadcaster_type,
      is_live: isLive,
      is_active: isActive,
    };
  }, [live?.title, live?.game_name, live?.viewer_count, live?.started_at, isLive, userInfo, channelKey, channelId, channelName, isActive]);

  // Hype Train: show this channel's train (Rust polls it once for every surface
  // showing it) and surface its start + each level-up in the combined activity
  // panel, Golden Kappa flagged. Only while the channel is live, since trains
  // only run on live channels. The per-train+level event id dedups, so a restart
  // (e.g. on go-live) never double-posts a level already seen.
  const hypeChannelId = channelId || userInfo?.id || '';
  // Per-pane hype train: drives both the in-pane progress banner (passed to
  // ChatWidget) and the combined activity-panel start/level-up rows.
  const [paneHypeTrain, setPaneHypeTrain] = useState<HypeTrainData | null>(null);
  // Clear any stale hype train the moment the channel goes offline. Adjusted during
  // render (React's supported alternative to a setState-in-effect for syncing state
  // to a value), not in an effect.
  if (!isLive && paneHypeTrain !== null) setPaneHypeTrain(null);
  useEffect(() => {
    if (!hypeChannelId || !isLive) return;
    const login = channelKey.toLowerCase();
    const display = channelName || channelKey;
    return watchHypeTrains([{ login, channelId: hypeChannelId, name: display }], (_login, train, levelChanged) => {
      setPaneHypeTrain(train);
      if (train && levelChanged) recordHypeTrainActivity(login, display, train);
    });
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
