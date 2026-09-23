// Open chat tabs for the mobile pane.
//
// Every tab holds its own reference on the shared IRC connection via
// chatConnectionStore's `acquireChannel`/`releaseChannel`, which is
// reference-counted and supports N concurrent channels off one socket. That is
// what makes switching tabs instant: background tabs stay connected and keep
// accumulating messages, so a switch is a re-render, not a reconnect.
//
// `useTwitchChat` is deliberately NOT used here. Its own header says new
// multi-channel code should talk to the store directly, because that wrapper
// tracks a single "current channel" and releases the previous one on switch,
// which is exactly the behaviour multi-chat must not have.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import type { ProviderId } from '../../types/providers';
import { normalizeChannel, sliceLookupKey } from '../../utils/providerKey';
import { Logger } from '../../utils/logger';

export interface ChatTab {
  /** Slice key in chatConnectionStore's space: bare lowercased login for Twitch,
   *  `kick:slug` otherwise. Unique per tab, and what useChannelChat reads. */
  channel: string;
  /** Platform the room is on. */
  provider: ProviderId;
  /** The bare name the platform addresses the room by (login or slug). What
   *  acquire/release, emote loads and sending take. */
  login: string;
  /** Numeric TWITCH id, for Helix mod actions, points, follow and pins. Always
   *  null on other platforms: a Kick id is numeric too, and passed to Helix it
   *  names an unrelated Twitch account. */
  channelId: string | null;
  /** Display name for the tab label. */
  label: string;
  /** Channel avatar for the tab. Absent falls back to an initial. */
  avatar?: string | null;
  /** True for the tab that follows the stream being watched. It is not
   *  removable and it re-points when the user switches streams. */
  pinnedToStream: boolean;
}

interface ChatTabsState {
  tabs: ChatTab[];
  activeChannel: string | null;
  /** Bumped per channel to force a chat reload. */
  reloadNonce: Record<string, number>;

  /** Point the stream-following tab at a new channel. */
  syncStreamTab: (
    channel: string | null,
    channelId: string | null,
    label: string,
    avatar?: string | null,
    provider?: ProviderId,
  ) => void;
  addTab: (
    channel: string,
    channelId: string | null,
    label: string,
    avatar?: string | null,
    provider?: ProviderId,
  ) => void;
  removeTab: (channel: string) => void;
  /** Fill in a tab's label or avatar after it opened (e.g. from a pasted link). */
  patchTab: (channel: string, patch: Partial<Pick<ChatTab, 'label' | 'avatar'>>) => void;
  setActive: (channel: string) => void;
  reload: (channel: string) => void;
}

/** One place that turns (name, id, provider) into a tab's identity. */
function tabIdentity(channel: string, channelId: string | null, provider: ProviderId) {
  const login = normalizeChannel(provider, channel.trim());
  return {
    key: sliceLookupKey(provider, login),
    login,
    channelId: provider === 'twitch' ? channelId : null,
  };
}

export const useChatTabsStore = create<ChatTabsState>((set, get) => ({
  tabs: [],
  activeChannel: null,
  reloadNonce: {},

  syncStreamTab: (channel, channelId, label, avatar, provider = 'twitch') => {
    const { tabs, activeChannel } = get();
    const existingStreamTab = tabs.find((t) => t.pinnedToStream);

    if (!channel) {
      // Stream closed. Drop the pinned tab but keep any manually added chats,
      // so moderating several rooms survives closing the player.
      if (existingStreamTab) {
        void releaseChannel(existingStreamTab.login, existingStreamTab.provider).catch(() => {});
        const rest = tabs.filter((t) => !t.pinnedToStream);
        set({
          tabs: rest,
          activeChannel:
            activeChannel === existingStreamTab.channel ? (rest[0]?.channel ?? null) : activeChannel,
        });
      }
      return;
    }

    const { key, login, channelId: id } = tabIdentity(channel, channelId, provider);
    if (existingStreamTab?.channel === key) return;

    // If this channel is already open as a manually added tab, promote it
    // rather than opening a duplicate.
    const alreadyOpen = tabs.find((t) => t.channel === key);

    if (existingStreamTab) void releaseChannel(existingStreamTab.login, existingStreamTab.provider).catch(() => {});
    if (!alreadyOpen) {
      void acquireChannel(login, id, provider).catch((err) =>
        Logger.warn('[ChatTabs] acquire failed:', err),
      );
    }

    const next: ChatTab[] = [
      { channel: key, provider, login, channelId: id, label, avatar, pinnedToStream: true },
      ...tabs.filter((t) => !t.pinnedToStream && t.channel !== key),
    ];
    set({
      tabs: next,
      // Follow the stream on switch unless the user is sitting on another room.
      activeChannel:
        activeChannel && activeChannel !== existingStreamTab?.channel && activeChannel !== key
          ? activeChannel
          : key,
    });
  },

  addTab: (channel, channelId, label, avatar, provider = 'twitch') => {
    const { key, login, channelId: id } = tabIdentity(channel, channelId, provider);
    const { tabs } = get();
    if (tabs.some((t) => t.channel === key)) {
      set({ activeChannel: key });
      return;
    }
    void acquireChannel(login, id, provider).catch((err) =>
      Logger.warn('[ChatTabs] acquire failed:', err),
    );
    set({
      tabs: [...tabs, { channel: key, provider, login, channelId: id, label, avatar, pinnedToStream: false }],
      activeChannel: key,
    });
    // A room opened from a pasted link (or a search row without a picture)
    // knows only its slug. Ask the platform for the real name and avatar so the
    // tab reads like every other one instead of a lowercase slug and a letter.
    if (provider !== 'twitch' && (!avatar || label === login)) {
      void invoke<{ user_name?: string; profile_image_url?: string }>('provider_channel_meta', {
        provider,
        channel: login,
      })
        .then((meta) => {
          get().patchTab(key, {
            ...(meta?.user_name ? { label: meta.user_name } : {}),
            ...(meta?.profile_image_url ? { avatar: meta.profile_image_url } : {}),
          });
        })
        .catch((err) => Logger.debug('[ChatTabs] channel meta unavailable:', err));
    }
  },

  patchTab: (channel, patch) => {
    const key = channel.toLowerCase();
    if (!get().tabs.some((t) => t.channel === key)) return;
    set((s) => ({ tabs: s.tabs.map((t) => (t.channel === key ? { ...t, ...patch } : t)) }));
  },

  removeTab: (channel) => {
    const key = channel.toLowerCase();
    const { tabs, activeChannel } = get();
    const tab = tabs.find((t) => t.channel === key);
    if (!tab || tab.pinnedToStream) return;
    void releaseChannel(tab.login, tab.provider).catch(() => {});
    const rest = tabs.filter((t) => t.channel !== key);
    set({
      tabs: rest,
      activeChannel: activeChannel === key ? (rest[0]?.channel ?? null) : activeChannel,
    });
  },

  setActive: (channel) => set({ activeChannel: channel.toLowerCase() }),

  reload: (channel) => {
    const key = channel.toLowerCase();
    const tab = get().tabs.find((t) => t.channel === key);
    if (!tab) return;
    // Non-Twitch rooms are not on the Twitch IRC service, so the full cycle
    // below does nothing for them. Rebuild the room's own adapter instead.
    //
    // At the ADAPTER, not the page store: the store is reference-counted, and
    // the player holds its own reference on the watched room, so a release and
    // re-acquire there only moved a counter and never reconnected anything.
    // The adapter counts one claim per window, so dropping and retaking it
    // closes the room's socket and opens a fresh one. The page's slice stays
    // attached to the shared bridge throughout and simply resumes.
    if (tab.provider !== 'twitch') {
      void (async () => {
        try {
          await invoke('provider_chat_disconnect', { provider: tab.provider, channel: tab.login }).catch(
            () => {},
          );
          await invoke('provider_chat_connect', { provider: tab.provider, channel: tab.login });
        } catch (err) {
          Logger.warn('[ChatTabs] reload failed:', err);
        }
        set((s) => ({ reloadNonce: { ...s.reloadNonce, [key]: (s.reloadNonce[key] ?? 0) + 1 } }));
      })();
      return;
    }
    // Rebuild the whole chat service rather than just this room.
    //
    // Dropping and retaking the reference is only a PART and a re-JOIN, which
    // is the right shape when the connection is healthy and the wrong one
    // whenever someone actually reaches for this control. People reload because
    // messages stopped, and messages stop because the upstream connection died,
    // in which case there is nothing left to re-JOIN onto and the reload
    // appears to do nothing at all. See chatRecovery for why only a full stop
    // clears that state.
    //
    // Forced, because a deliberate tap must always act even if something else
    // rebuilt the connection moments ago. hardCycleChat bumps the nonce for
    // every room it touches, so this one gets refreshed along the way.
    //
    // Imported here rather than at the top because chatRecovery reads this
    // store, and a static import both ways is a cycle.
    void (async () => {
      try {
        const { hardCycleChat } = await import('./chatRecovery');
        await hardCycleChat(`manual reload of ${key}`, true);
      } catch (err) {
        Logger.warn('[ChatTabs] reload failed:', err);
      }
    })();
  },
}));
