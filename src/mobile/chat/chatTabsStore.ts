// Open chat tabs: the strip under the player and the Chats screen.
//
// The tab rules live in Rust (`services/chat_tabs.rs`), shared with the iPhone
// app: the stream tab is first and not closable, a room already open is never
// opened twice, closing the room on screen falls back to the first one left,
// and the stream tab follows the player unless you sit on another room. Rooms
// you open yourself are your chats: Rust saves them per account, restores them
// at launch and joins them on the first look at the Chats screen (or when a
// stream starts). This store mirrors Rust's list (from each command's answer
// and the `chat-tabs` event, newest revision wins) and applies the connections
// each change asks for.
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
import { listen } from '@tauri-apps/api/event';
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import type { ProviderId } from '../../types/providers';
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
  /** One of your chats: saved, and kept when the player closes. Every tab
   *  you opened is kept; the stream tab only when you chose to keep it. */
  kept: boolean;
  /** Live now; null until the room's live line has answered. */
  live: boolean | null;
  viewerCount: number | null;
  /** Mentions of you here not read yet. */
  mentions: number;
}

/** Whether the rooms are joined: restored and waiting for a first look,
 *  joined, or left after a while in the background. */
export type ChatConnection = 'idle' | 'connected' | 'parked';

/** A tab as Rust sends it. */
interface RustTab {
  channel: string;
  provider: ProviderId;
  login: string;
  channel_id: string | null;
  label: string;
  avatar: string | null;
  pinned_to_stream: boolean;
  kept?: boolean;
  live?: boolean | null;
  viewer_count?: number | null;
  mentions?: number;
}

interface RustView {
  tabs: RustTab[];
  active_channel: string | null;
  mixed_platforms: boolean;
  connection?: ChatConnection;
  mention_total?: number;
  park_after_secs?: number;
  keep_awake_idle_secs?: number;
  revision: number;
}

interface RustChange extends RustView {
  acquire: RustTab[];
  release: RustTab[];
}

interface ChatTabsState {
  tabs: ChatTab[];
  activeChannel: string | null;
  connection: ChatConnection;
  /** Unread mentions across every tab: the Chats tab's badge. */
  mentionTotal: number;
  /** Android leaves every room after this long in the background with no player. */
  parkAfterSecs: number;
  /** The Chats screen keeps the phone awake until this long passes with no touch. */
  keepAwakeIdleSecs: number;
  /** Rust's revision of the list shown, so an older answer never wins. */
  revision: number;
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
  setActive: (channel: string) => void;
  reload: (channel: string) => void;
  /** Reads the saved chats (at launch: Rust restores them, not joined yet). */
  load: () => void;
  /** Joins every chat not joined yet: the Chats screen's first look, or the
   *  return from a park. */
  connect: () => void;
  /** Leaves every chat after a while in the background (refused under a stream). */
  park: () => void;
  /** Keeps the stream's chat after the player closes, or lets it close with it. */
  keep: (channel: string, keep: boolean) => void;
}

function fromRust(t: RustTab): ChatTab {
  return {
    channel: t.channel,
    provider: t.provider,
    login: t.login,
    channelId: t.channel_id,
    label: t.label,
    avatar: t.avatar,
    pinnedToStream: t.pinned_to_stream,
    kept: t.kept ?? !t.pinned_to_stream,
    live: t.live ?? null,
    viewerCount: t.viewer_count ?? null,
    mentions: t.mentions ?? 0,
  };
}

function room(channel: string, channelId: string | null, label: string, avatar?: string | null, provider?: ProviderId) {
  return { provider: provider ?? 'twitch', channel, channel_id: channelId, label, avatar: avatar ?? null };
}

let listening = false;

export const useChatTabsStore = create<ChatTabsState>((set, get) => {
  /** Shows Rust's list unless a newer one is already on screen. */
  const mirror = (view: RustView | null | undefined) => {
    if (!view || view.revision < get().revision) return;
    set({
      tabs: view.tabs.map(fromRust),
      activeChannel: view.active_channel,
      connection: view.connection ?? 'connected',
      mentionTotal: view.mention_total ?? 0,
      parkAfterSecs: view.park_after_secs ?? get().parkAfterSecs,
      keepAwakeIdleSecs: view.keep_awake_idle_secs ?? get().keepAwakeIdleSecs,
      revision: view.revision,
    });
  };

  /** Mirrors the change, then leaves and joins the rooms it names. */
  const apply = (change: RustChange | null | undefined) => {
    if (!change) return;
    mirror(change);
    for (const t of change.release ?? []) {
      void releaseChannel(t.login, t.provider).catch(() => {});
    }
    for (const t of change.acquire ?? []) {
      void acquireChannel(t.login, t.channel_id, t.provider).catch((err) =>
        Logger.warn('[ChatTabs] acquire failed:', err),
      );
    }
  };

  // Rust patches a tab on its own (a Kick room's real name and picture), so
  // follow its event too. Started on the first change, so tests and boot order
  // need nothing set up.
  const follow = () => {
    if (listening) return;
    listening = true;
    void Promise.resolve(listen<RustView>('chat-tabs', (e) => mirror(e.payload))).catch((err) =>
      Logger.debug('[ChatTabs] no tab events:', err),
    );
  };

  const change = (command: string, args: Record<string, unknown>) => {
    follow();
    void invoke<RustChange>(command, args)
      .then(apply)
      .catch((err) => Logger.warn(`[ChatTabs] ${command} failed:`, err));
  };

  return {
    tabs: [],
    activeChannel: null,
    connection: 'idle',
    mentionTotal: 0,
    parkAfterSecs: 120,
    keepAwakeIdleSecs: 1800,
    revision: 0,
    reloadNonce: {},

    load: () => {
      follow();
      void invoke<RustView>('chat_tabs_get')
        .then(mirror)
        .catch((err) => Logger.warn('[ChatTabs] chat_tabs_get failed:', err));
    },

    connect: () => change('chat_tabs_connect', {}),

    park: () => change('chat_tabs_park', {}),

    keep: (channel, keep) => {
      follow();
      void invoke<RustView>('chat_tabs_keep', { channel, keep })
        .then(mirror)
        .catch((err) => Logger.warn('[ChatTabs] chat_tabs_keep failed:', err));
    },

    syncStreamTab: (channel, channelId, label, avatar, provider) =>
      change('chat_tabs_sync_stream', {
        room: channel ? room(channel, channelId, label, avatar, provider) : null,
      }),

    addTab: (channel, channelId, label, avatar, provider) =>
      change('chat_tabs_add', { room: room(channel, channelId, label, avatar, provider) }),

    removeTab: (channel) => change('chat_tabs_remove', { channel }),

    setActive: (channel) => {
      follow();
      // Shown at once; Rust's answer confirms it (or puts back a room that is gone).
      const key = channel.toLowerCase();
      if (get().tabs.some((t) => t.channel === key)) set({ activeChannel: key });
      void invoke<RustView>('chat_tabs_set_active', { channel })
        .then(mirror)
        .catch((err) => Logger.warn('[ChatTabs] chat_tabs_set_active failed:', err));
    },

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
      // rebuilt the connection moments ago. Every room keeps its rows through
      // the rebuild, and its history load fills what the outage missed.
      //
      // Imported here rather than at the top to keep the recovery module out
      // of this store's load.
      void (async () => {
        try {
          const { hardCycleChat } = await import('./chatRecovery');
          await hardCycleChat(`manual reload of ${key}`, true);
        } catch (err) {
          Logger.warn('[ChatTabs] reload failed:', err);
        }
      })();
    },
  };
});
