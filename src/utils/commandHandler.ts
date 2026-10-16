// Slash commands run in Rust: `run_slash_command` (services/slash_dispatch.rs)
// parses the line, does the work and answers what the composer does next.
// This file only applies that answer: the toasts, the page actions Rust cannot
// take (slashUi.ts), the lines printed in the room, and the text to send.

import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { usePinStore } from '../stores/pinStore';
import { Logger } from './logger';
import { IS_ANDROID, IS_MOBILE } from './platform';
import { applySlashUi, noticeIn, type SlashUi } from './slashUi';

export { noticeIn } from './slashUi';

/** The answer of `run_slash_command`. */
export interface SlashOutcome {
  /** False: not a command StreamNook runs; send the text as typed. */
  handled: boolean;
  /** Text to send as an ordinary chat message in the room. */
  send: string | null;
  /** The parent message `send` replies to (/reply). */
  reply_to?: string;
  /** Pin `send` once Twitch has given it an id (/pin <message>). */
  pin_after_send?: boolean;
  clear_draft: boolean;
  toasts: { text: string; kind: 'success' | 'error' | 'info' }[];
  /** Lines printed in this room's chat that only you see. */
  notices: string[];
  ui: SlashUi | null;
}

const PLATFORM = IS_ANDROID ? 'android' : IS_MOBILE ? 'ios' : 'desktop';

/** Ask Rust to run `text` typed in `channel` (a bare Twitch login, or a
 *  provider room key such as `kick:slug`). */
export function runSlashCommand(text: string, broadcasterId: string, channel: string, depth = 0): Promise<SlashOutcome> {
  return invoke<SlashOutcome>('run_slash_command', {
    channel,
    broadcasterId,
    text,
    depth,
    platform: PLATFORM,
  });
}

/** Send `text` with a reply parent or a pin, through the chat store's own send
 *  path (the optimistic row and its reconciliation live there). */
async function sendSpecial(channel: string, text: string, replyTo: string | undefined, pin: boolean): Promise<void> {
  const addToast = useAppStore.getState().addToast;
  const user = useAppStore.getState().currentUser;
  if (!user) {
    addToast(pin ? '/pin: sign in to send a message' : 'Sign in to Twitch to reply', 'error');
    return;
  }
  const { sendChannelMessage } = await import('../stores/chatConnectionStore');
  const info = {
    username: user.login || user.username,
    displayName: user.display_name || user.username,
    userId: user.user_id,
    color: undefined,
    badges: '',
  };
  try {
    const outcome = await sendChannelMessage(channel, text, info, replyTo, null, pin ? { pinAfterSend: true } : undefined);
    if (!outcome) {
      addToast(pin ? '/pin: chat is not connected' : 'Chat is not connected', 'error');
      return;
    }
    // A dropped message is not pinned. Desktop prints Twitch's reason in the
    // room; the phone prints no row for it, so it is said here.
    if (!outcome.is_sent) {
      if (IS_MOBILE && outcome.drop_reason) addToast(`Not sent: ${outcome.drop_reason}`, 'error');
      else if (pin && !outcome.drop_reason) addToast('Message not sent, so nothing was pinned', 'error');
      return;
    }
    if (!pin) return;
    if (outcome.pin_error) {
      addToast(outcome.pin_error, 'error');
      return;
    }
    usePinStore.getState().requestRefresh();
    addToast('Sent and pinned', 'success');
  } catch (err: unknown) {
    addToast(typeof err === 'string' ? err : pin ? '/pin failed' : 'Could not send the reply', 'error');
  }
}

/** Apply an answer in the room it was typed in. */
export async function applySlashOutcome(
  outcome: SlashOutcome,
  channel: string,
  broadcasterId: string,
  sendMessageToChat: (msg: string) => void,
): Promise<void> {
  const addToast = useAppStore.getState().addToast;
  for (const t of outcome.toasts) addToast(t.text, t.kind);
  // Before the lines, so /clearmessages prints into the emptied chat and
  // /reconnect says so after the reload.
  if (outcome.ui) await applySlashUi(outcome.ui, channel, broadcasterId);
  for (const line of outcome.notices) noticeIn(channel, line);
  if (outcome.send != null) {
    if (outcome.reply_to || outcome.pin_after_send) {
      await sendSpecial(channel, outcome.send, outcome.reply_to, !!outcome.pin_after_send);
    } else {
      sendMessageToChat(outcome.send);
    }
  }
}

/**
 * Run a typed line through Rust and apply the answer. True when it was a
 * command (the caller clears the box); false when the caller should send the
 * text as typed.
 */
export const handleSlashCommand = async (
  message: string,
  broadcasterId: string,
  broadcasterLogin: string,
  sendMessageToChat: (msg: string) => void,
  depth = 0,
): Promise<boolean> => {
  let outcome: SlashOutcome;
  try {
    outcome = await runSlashCommand(message, broadcasterId, broadcasterLogin, depth);
  } catch (err: unknown) {
    const errMsg = err instanceof Error ? err.message : String(err);
    Logger.error('[Command Handler] run_slash_command failed:', err);
    useAppStore.getState().addToast(`Command failed: ${errMsg}`, 'error');
    return true;
  }
  if (!outcome.handled) return false;
  await applySlashOutcome(outcome, broadcasterLogin, broadcasterId, sendMessageToChat);
  return true;
};
