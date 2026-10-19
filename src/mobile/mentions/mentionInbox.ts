// The phone's view of the mentions inbox. Rust owns the inbox (who mentioned
// you, which are read, what a moderator removed) and the rules that file into
// it; this module only asks for pages, relays the row actions, and holds the
// two pieces of page state a jump needs: the entry being looked for in a chat,
// and the "Older than chat history" card when it could not be found.
//
// Notifications are not here: Rust posts them itself (android_notify.rs),
// because wry pauses the WebView whenever the app leaves the screen.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { useMobileNavStore } from '../navStore';
import { Logger } from '../../utils/logger';
import type { ProviderId } from '../../types/providers';
import type { MentionEntry, MentionPage } from '../../types';

export type { MentionEntry, MentionPage };

export const PAGE_SIZE = 40;

/** What a row says about why it is here. */
export const KIND_LABEL: Record<MentionEntry['kind'], string> = {
  mention: 'mentioned you',
  reply: 'replied to you',
  word: 'highlight',
};

/** A room key's platform: `kick:xqc` is Kick, a bare login is Twitch. */
export function roomProvider(room: string): ProviderId {
  const i = room.indexOf(':');
  return (i > 0 ? room.slice(0, i) : 'twitch') as ProviderId;
}

/** A room key's channel name, without the platform. */
export function roomName(room: string): string {
  const i = room.indexOf(':');
  return i > 0 ? room.slice(i + 1) : room;
}

interface MentionsState {
  unreadTotal: number;
  /** Bumped on every `mention-inbox-changed`, so an open list re-reads. */
  revision: number;
  /** The entry an opened chat is looking for, until it is found or given up. */
  pendingJump: MentionEntry | null;
  /** Shown pinned over its chat when the message was older than its history. */
  contextCard: MentionEntry | null;
}

export const useMentionsStore = create<MentionsState>(() => ({
  unreadTotal: 0,
  revision: 0,
  pendingJump: null,
  contextCard: null,
}));

export function dismissContextCard(): void {
  useMentionsStore.setState({ contextCard: null });
}

export function getMentionPage(beforeMs?: number, limit = PAGE_SIZE): Promise<MentionPage> {
  return invoke<MentionPage>('get_mention_inbox', { beforeMs, limit });
}

function quietly(command: string, args?: Record<string, unknown>): Promise<void> {
  return invoke(command, args).then(
    () => undefined,
    (err) => {
      Logger.warn(`[Mentions] ${command} failed:`, err);
      useAppStore.getState().addToast('Could not update your mentions.', 'error');
    },
  );
}

export const markAllMentionsRead = () => quietly('mark_mentions_read', {});
export const removeMention = (id: string) => quietly('remove_mention', { id });
export const clearMentionInbox = () => quietly('clear_mention_inbox');

function mutedRooms(): string[] {
  return useAppStore.getState().settings.mentions?.muted_channels ?? [];
}

export function isRoomMuted(room: string): boolean {
  return mutedRooms().includes(room.toLowerCase());
}

/** Adds or takes a chat off `mentions.muted_channels`. Rust stops filing and
 *  announcing mentions there the moment the setting lands. */
export async function setRoomMuted(room: string, muted: boolean): Promise<void> {
  const key = room.trim().toLowerCase();
  if (!key || mutedRooms().includes(key) === muted) return;
  try {
    // One chat per edit, applied by Rust to the stored set, so a list another
    // window just changed is never written back over.
    await invoke('edit_settings_list', {
      path: 'mentions.muted_channels',
      edit: { op: 'toggle', value: key },
    });
  } catch (err) {
    Logger.warn('[Mentions] mute change failed:', err);
    useAppStore.getState().addToast('Could not save that change.', 'error');
  }
}

/**
 * Open a mention's chat and look for the message there.
 *
 * Watching something already: the room opens as a chat tab beside the stream
 * (or comes to the front, when it is the stream's own chat), so playback is not
 * interrupted. Watching nothing: the channel opens the way any channel does.
 * The chat pane then finds the message, or backfills once, or pins the card.
 */
export function openMentionEntry(entry: MentionEntry): void {
  const app = useAppStore.getState();
  const nav = useMobileNavStore.getState();
  nav.setMentionsOpen(false);
  useMentionsStore.setState({ pendingJump: entry, contextCard: null });
  const watching = !!(app.streamUrl || app.isLoading);
  if (watching) {
    const provider = roomProvider(entry.room);
    useChatTabsStore
      .getState()
      .addTab(
        entry.channel_login || roomName(entry.room),
        provider === 'twitch' ? (entry.channel_id ?? null) : null,
        entry.channel_login || roomName(entry.room),
        null,
        provider,
      );
    nav.setPlayerMode('full');
  } else {
    nav.setPlayerMode('full');
    void app.startStream(entry.open_key || entry.room);
  }
}

/** A notification tap: Rust marks the entry read and hands it back. Right
 *  after a cold start the account may not be known yet, so a miss retries a
 *  few times before giving up. */
export async function openMentionById(id: string, attempts = 6): Promise<void> {
  for (let i = 0; i < attempts; i++) {
    try {
      const entry = await invoke<MentionEntry | null>('open_mention', { id });
      if (entry) {
        openMentionEntry(entry);
        return;
      }
    } catch (err) {
      Logger.warn('[Mentions] open_mention failed:', err);
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
}

/** A row tap. */
export async function openMention(entry: MentionEntry): Promise<void> {
  try {
    const fresh = await invoke<MentionEntry | null>('open_mention', { id: entry.id });
    openMentionEntry(fresh ?? entry);
  } catch (err) {
    Logger.warn('[Mentions] open_mention failed:', err);
    openMentionEntry(entry);
  }
}

let installed = false;

/** Mounted once by the shell: the unread count, seeded now and kept current by
 *  Rust's change event. Re-seeded on sign-in, since the inbox is per account. */
export function installMentionInbox(): () => void {
  if (installed) return () => {};
  installed = true;
  const seed = () => {
    void getMentionPage(undefined, 1)
      .then((p) => useMentionsStore.setState({ unreadTotal: p.unread_total }))
      .catch(() => {});
  };
  seed();
  let unlisten: (() => void) | null = null;
  let dead = false;
  void listen<{ unread_total: number }>('mention-inbox-changed', (e) => {
    useMentionsStore.setState((s) => ({
      unreadTotal: e.payload?.unread_total ?? s.unreadTotal,
      revision: s.revision + 1,
    }));
  }).then((u) => {
    if (dead) u();
    else unlisten = u;
  });
  const unsubUser = useAppStore.subscribe((s, prev) => {
    if (s.currentUser?.user_id !== prev.currentUser?.user_id) seed();
  });
  // The context card belongs to its chat: leaving that chat dismisses it.
  const unsubTabs = useChatTabsStore.subscribe((s, prev) => {
    if (s.activeChannel === prev.activeChannel) return;
    const card = useMentionsStore.getState().contextCard;
    if (card && card.room !== s.activeChannel) dismissContextCard();
  });
  return () => {
    dead = true;
    installed = false;
    unlisten?.();
    unsubUser();
    unsubTabs();
  };
}
