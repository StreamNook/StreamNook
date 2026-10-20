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
import type { InboxFilterId, InboxRowView, InboxView, MentionArrival, MentionEntry, MentionPage } from '../../types';

export type { InboxFilterId, InboxRowView, InboxView, MentionArrival, MentionEntry, MentionPage };

/** Where an inbox sheet opens: the chat under the player, or the Chats tab. */
export type InboxSurface = 'watch' | 'chats';

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
  /** The chat whose inbox sheet is open, if any. */
  sheetIn: InboxSurface | null;
  /** The latest arrival waiting for, or showing as, the card over the bar. */
  arrival: (MentionArrival & { seq: number }) | null;
  /** The composer holds a long draft or its suggestions strip: an arrival
   *  waits rather than sit over what is being written. */
  composerBusy: boolean;
}

export const useMentionsStore = create<MentionsState>(() => ({
  unreadTotal: 0,
  revision: 0,
  pendingJump: null,
  contextCard: null,
  sheetIn: null,
  arrival: null,
  composerBusy: false,
}));

/** The tray's tap: open this chat's sheet, or close it when it is open. An
 *  arrival card folds away, since the sheet now shows it. */
export function toggleInboxSheet(surface: InboxSurface): void {
  useMentionsStore.setState((s) => ({ sheetIn: s.sheetIn === surface ? null : surface, arrival: null }));
}

export function closeInboxSheet(): void {
  if (useMentionsStore.getState().sheetIn) useMentionsStore.setState({ sheetIn: null });
}

export function dismissArrival(seq: number): void {
  if (useMentionsStore.getState().arrival?.seq === seq) useMentionsStore.setState({ arrival: null });
}

export function setComposerBusy(busy: boolean): void {
  if (useMentionsStore.getState().composerBusy !== busy) useMentionsStore.setState({ composerBusy: busy });
}

/** One page of the timeline for a chip: Rust filters, folds identical
 *  messages and attaches the senders' pictures it has. */
export function getInboxView(filter: InboxFilterId, beforeMs?: number | null, limit = PAGE_SIZE): Promise<InboxView> {
  return invoke<InboxView>('get_inbox_view', { filter, beforeMs: beforeMs ?? undefined, limit });
}

/** Rows the sheet showed while it was open, marked read as it closes. */
export function markInboxSeen(ids: string[]): void {
  if (ids.length === 0) return;
  void invoke('mark_mentions_read', { ids }).catch((err) => Logger.warn('[Inbox] mark seen failed:', err));
}

/**
 * Hooks the whisper inbox fills in when it joins the timeline (Rust adds the
 * `whispers` chip and `whisper` rows to `get_inbox_view`): what a whisper row
 * opens. Until then no whisper row exists, so nothing calls it.
 */
export const inboxHooks: { openWhisper: ((row: InboxRowView) => void) | null } = { openWhisper: null };

/** A row tap: jump to the message (its whole fold reads as read), or open the
 *  whisper conversation. */
export function openInboxRow(row: InboxRowView): void {
  if ((row.entry.kind as string) === 'whisper') {
    inboxHooks.openWhisper?.(row);
    return;
  }
  closeInboxSheet();
  const folded = row.folded.filter((f) => !f.read).map((f) => f.id);
  if (folded.length) void invoke('mark_mentions_read', { ids: folded }).catch(() => {});
  void openMention(row.entry);
}

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

/** The picture cache's key for a sender, as Rust's `avatar_request` builds it:
 *  Twitch and YouTube ids, Kick slugs. */
function arrivalAvatarKey(e: MentionEntry): string | null {
  const id = e.platform === 'kick' ? e.sender_login : e.platform === 'twitch' || e.platform === 'youtube' ? e.sender_id : '';
  return id ? `${e.platform}:${id.toLowerCase()}` : null;
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
  let dead = false;
  const unlistens: (() => void)[] = [];
  const keep = (p: Promise<() => void>) =>
    void p.then((u) => {
      if (dead) u();
      else unlistens.push(u);
    });
  keep(
    listen<{ unread_total: number }>('mention-inbox-changed', (e) => {
      useMentionsStore.setState((s) => ({
        unreadTotal: e.payload?.unread_total ?? s.unreadTotal,
        revision: s.revision + 1,
      }));
    }),
  );
  // A new entry for the card over the bar. Rust already left out replay, a
  // chat on screen and streamer mode. With the sheet open the sheet shows it.
  let seq = 0;
  keep(
    listen<MentionArrival>('mention-arrived', (e) => {
      if (!e.payload?.entry) return;
      Logger.debug('[Inbox] arrival', e.payload.entry.id);
      if (useMentionsStore.getState().sheetIn) return;
      useMentionsStore.setState({ arrival: { ...e.payload, seq: ++seq } });
    }),
  );
  // Senders' pictures landing: an open inbox reads again to show them, and a
  // card still up takes its sender's.
  keep(
    listen<Record<string, string>>('channel-avatars', (e) => {
      const s = useMentionsStore.getState();
      const a = s.arrival;
      if (a && !a.avatar) {
        const key = arrivalAvatarKey(a.entry);
        const url = key ? e.payload?.[key] : undefined;
        if (url) useMentionsStore.setState({ arrival: { ...a, avatar: url } });
      }
      if (s.sheetIn || useMobileNavStore.getState().mentionsOpen) {
        useMentionsStore.setState((st) => ({ revision: st.revision + 1 }));
      }
    }),
  );
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
    unlistens.forEach((u) => u());
    unsubUser();
    unsubTabs();
  };
}
