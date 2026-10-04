// The chats docked in the main window, mirrored from Rust
// (src-tauri/src/commands/chat_dock.rs), which owns the list so it survives the
// main window being rebuilt and the app restarting. This store never decides
// anything Rust stores; it holds the copy the page renders from, whether the
// slide-out list is open, and the one rule that is pure presentation: which
// chat is on screen right now.
//
// Not combined chat (a streamer's own other-platform chats), and not a pinned
// MESSAGE (channel_state's `pinned`).
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ProviderId } from '../types/providers';
import { makeKey } from '../utils/providerKey';
import { Logger } from '../utils/logger';

export interface DockedChat {
  provider: ProviderId;
  login: string;
  channel_id: string;
  display_name: string;
  avatar_url: string | null;
  /** Mark the chat on any new message while hidden; off marks mentions only. */
  light_on_new: boolean;
}

interface ChatDockMirror {
  chats: DockedChat[];
  /** The docked chat on screen, by `makeKey`; null shows the watched stream's. */
  active: string | null;
}

/** A docked Kick / YouTube / TikTok chat's live line, from Rust
 *  (services/chat_dock_live.rs); Twitch ones read channel state. */
export interface DockLive {
  live: boolean;
  viewer_count: number | null;
  category: string | null;
  title: string | null;
}

interface ChatDockState extends ChatDockMirror {
  /** The chat list is open. */
  panelOpen: boolean;
  /** By `makeKey`, for docked chats on platforms other than Twitch. */
  live: Record<string, DockLive>;
}

export const useChatDockStore = create<ChatDockState>(() => ({ chats: [], active: null, panelOpen: false, live: {} }));

let started = false;

/** Load the dock and follow Rust's updates. Main window only, once. */
export function startChatDock(): void {
  if (started) return;
  started = true;
  void listen<ChatDockMirror>('chat-dock', (event) => useChatDockStore.setState(event.payload));
  void listen<Record<string, DockLive>>('chat-dock-live', (event) => useChatDockStore.setState({ live: event.payload }));
  invoke<ChatDockMirror>('get_chat_dock')
    .then((s) => useChatDockStore.setState(s))
    .catch((e: unknown) => Logger.warn('[ChatDock] load failed:', e));
  invoke<Record<string, DockLive>>('get_chat_dock_live')
    .then((live) => useChatDockStore.setState({ live }))
    .catch((e: unknown) => Logger.warn('[ChatDock] live load failed:', e));
}

function apply(promise: Promise<ChatDockMirror>): Promise<void> {
  return promise
    .then((s) => useChatDockStore.setState(s))
    .catch((e: unknown) => Logger.warn('[ChatDock] update failed:', e));
}

export function dockChat(chat: Omit<DockedChat, 'light_on_new'>, show: boolean): Promise<void> {
  return apply(invoke<ChatDockMirror>('dock_chat', { chat: { ...chat, light_on_new: true }, show }));
}

export function undockChat(key: string): Promise<void> {
  return apply(invoke<ChatDockMirror>('undock_chat', { key }));
}

/** Show a docked chat, or the watched stream's with null. */
export function showDockedChat(key: string | null): Promise<void> {
  return apply(invoke<ChatDockMirror>('show_docked_chat', { key }));
}

export function reorderChatDock(keys: string[]): Promise<void> {
  return apply(invoke<ChatDockMirror>('reorder_chat_dock', { keys }));
}

export function setDockedChatLight(key: string, on: boolean): Promise<void> {
  return apply(invoke<ChatDockMirror>('set_docked_chat_light', { key, on }));
}

export function setDockPanelOpen(open: boolean): void {
  useChatDockStore.setState({ panelOpen: open });
}

/** The channel a chat shows, as far as docking cares. */
export interface ChatChannel {
  provider: ProviderId;
  login: string;
}

export function chatKey(c: ChatChannel): string {
  return makeKey(c.provider, c.login);
}

export interface DockView {
  /** The dock is in play (it sleeps under MultiNook). */
  held: boolean;
  /** The docked chat on screen; null means the watched stream's chat. */
  shown: DockedChat | null;
  /** Docked chats other than the watched stream's own. */
  others: DockedChat[];
}

/**
 * Which chat is on screen. MultiNook picks its chat among tiles, so the dock
 * sleeps there. A docked chat that is the stream being watched is that
 * stream's chat, not a second copy of it. With no stream playing, the chosen
 * docked chat shows, or the first one.
 */
export function dockView(
  chats: DockedChat[],
  active: string | null,
  live: ChatChannel | null,
  multiNookActive: boolean,
): DockView {
  if (multiNookActive || chats.length === 0) return { held: false, shown: null, others: [] };
  const liveKey = live ? chatKey(live) : null;
  const others = chats.filter((c) => chatKey(c) !== liveKey);
  const chosen = active ? others.find((c) => chatKey(c) === active) ?? null : null;
  const shown = chosen ?? (live ? null : others[0] ?? null);
  return { held: true, shown, others };
}
