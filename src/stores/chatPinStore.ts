// The chat pinned in the main window, mirrored from Rust
// (src-tauri/src/commands/chat_pin.rs), which owns it so it survives the main
// window being rebuilt and the app restarting. This store never decides
// anything Rust stores; it holds the copy the page renders from and the one
// rule that is pure presentation: whether the pinned and live chats are two
// different chats right now, and which of them is on screen.
//
// Not the same thing as a pinned MESSAGE (channel_state's `pinned`).
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ProviderId } from '../types/providers';
import { makeKey } from '../utils/providerKey';
import { Logger } from '../utils/logger';

export interface ChatPin {
  provider: ProviderId;
  login: string;
  channel_id: string;
  display_name: string;
  avatar_url: string | null;
}

export type ChatPinView = 'pinned' | 'live';

interface ChatPinState {
  pin: ChatPin | null;
  view: ChatPinView;
}

export const useChatPinStore = create<ChatPinState>(() => ({ pin: null, view: 'live' }));

let started = false;

/** Load the pin and follow Rust's updates. Main window only, once. */
export function startChatPin(): void {
  if (started) return;
  started = true;
  void listen<ChatPinState>('chat-pin', (event) => useChatPinStore.setState(event.payload));
  invoke<ChatPinState>('get_chat_pin')
    .then((s) => useChatPinStore.setState(s))
    .catch((e: unknown) => Logger.warn('[ChatPin] load failed:', e));
}

function apply(promise: Promise<ChatPinState>): Promise<void> {
  return promise
    .then((s) => useChatPinStore.setState(s))
    .catch((e: unknown) => Logger.warn('[ChatPin] update failed:', e));
}

export function pinChat(pin: ChatPin): Promise<void> {
  return apply(invoke<ChatPinState>('set_chat_pin', { pin }));
}

export function unpinChat(): Promise<void> {
  return apply(invoke<ChatPinState>('set_chat_pin', { pin: null }));
}

export function showChatPinSide(view: ChatPinView): Promise<void> {
  return apply(invoke<ChatPinState>('set_chat_pin_view', { view }));
}

/** The channel a chat shows, as far as pinning cares. */
export interface ChatChannel {
  provider: ProviderId;
  login: string;
}

export function chatKey(c: ChatChannel): string {
  return makeKey(c.provider, c.login);
}

export interface PinPair {
  /** A pin exists and it is a different chat from the live one, so the main
   *  window holds both and offers the switch. */
  split: boolean;
  /** The pinned chat is the one on screen. */
  showPinned: boolean;
  /** The pin is in play at all (not dormant under MultiNook). */
  held: boolean;
}

/**
 * Whether the pinned and live chats are two chats right now, and which one is
 * on screen. MultiNook picks its chat among tiles, so the pin sleeps there.
 * With no live stream the pinned chat is the only one.
 */
export function pinPair(
  pin: ChatPin | null,
  view: ChatPinView,
  live: ChatChannel | null,
  multiNookActive: boolean,
): PinPair {
  if (!pin || multiNookActive) return { split: false, showPinned: false, held: false };
  if (!live) return { split: false, showPinned: true, held: true };
  const split = chatKey(pin) !== chatKey(live);
  return { split, showPinned: split && view === 'pinned', held: true };
}
