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
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import { Logger } from '../../utils/logger';

export interface ChatTab {
  /** Lowercased login; the channel key used by the connection store. */
  channel: string;
  /** Numeric Twitch id, needed for mod actions and emote loads. */
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
  ) => void;
  addTab: (
    channel: string,
    channelId: string | null,
    label: string,
    avatar?: string | null,
  ) => void;
  removeTab: (channel: string) => void;
  setActive: (channel: string) => void;
  reload: (channel: string) => void;
}

export const useChatTabsStore = create<ChatTabsState>((set, get) => ({
  tabs: [],
  activeChannel: null,
  reloadNonce: {},

  syncStreamTab: (channel, channelId, label, avatar) => {
    const { tabs, activeChannel } = get();
    const existingStreamTab = tabs.find((t) => t.pinnedToStream);

    if (!channel) {
      // Stream closed. Drop the pinned tab but keep any manually added chats,
      // so moderating several rooms survives closing the player.
      if (existingStreamTab) {
        void releaseChannel(existingStreamTab.channel).catch(() => {});
        const rest = tabs.filter((t) => !t.pinnedToStream);
        set({
          tabs: rest,
          activeChannel:
            activeChannel === existingStreamTab.channel ? (rest[0]?.channel ?? null) : activeChannel,
        });
      }
      return;
    }

    const key = channel.toLowerCase();
    if (existingStreamTab?.channel === key) return;

    // If this channel is already open as a manually added tab, promote it
    // rather than opening a duplicate.
    const alreadyOpen = tabs.find((t) => t.channel === key);

    if (existingStreamTab) void releaseChannel(existingStreamTab.channel).catch(() => {});
    if (!alreadyOpen) {
      void acquireChannel(key, channelId).catch((err) =>
        Logger.warn('[ChatTabs] acquire failed:', err),
      );
    }

    const next: ChatTab[] = [
      { channel: key, channelId, label, avatar, pinnedToStream: true },
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

  addTab: (channel, channelId, label, avatar) => {
    const key = channel.toLowerCase();
    const { tabs } = get();
    if (tabs.some((t) => t.channel === key)) {
      set({ activeChannel: key });
      return;
    }
    void acquireChannel(key, channelId).catch((err) =>
      Logger.warn('[ChatTabs] acquire failed:', err),
    );
    set({
      tabs: [...tabs, { channel: key, channelId, label, avatar, pinnedToStream: false }],
      activeChannel: key,
    });
  },

  removeTab: (channel) => {
    const key = channel.toLowerCase();
    const { tabs, activeChannel } = get();
    const tab = tabs.find((t) => t.channel === key);
    if (!tab || tab.pinnedToStream) return;
    void releaseChannel(key).catch(() => {});
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
