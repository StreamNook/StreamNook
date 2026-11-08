// The words every surface shows for one of our own whispers on its way to
// Twitch. Rust decides the state (services/whisper_send.rs); this only names it.
import type { StoredWhisperMessage, WhisperDelivery } from '../types';

const KNOWN: ReadonlySet<string> = new Set(['sending', 'sent', 'unknown', 'delivered', 'not_delivered']);

/** Where a message stands: null when it is not one of our own sends. A state
 *  this build does not know reads as Sent. */
export function deliveryOf(m: Pick<StoredWhisperMessage, 'delivery'> | { delivery: string | null }): WhisperDelivery | null {
  const value = m.delivery;
  if (value == null) return null;
  return KNOWN.has(value) ? (value as WhisperDelivery) : 'sent';
}

export const DELIVERY_LABEL: Record<WhisperDelivery, string> = {
  sending: 'Sending...',
  sent: 'Sent',
  unknown: 'Checking...',
  delivered: 'Delivered',
  not_delivered: 'Not delivered',
};

/** The line under a whisper Twitch shortened. */
export function cutLine(characters: number): string {
  return `Twitch cut this whisper to ${characters} characters.`;
}

/** Why a notification reply was refused after an account switch. */
export function replyRefused(name: string): string {
  return `This whisper came to ${name}. Switch to that account to reply.`;
}

/** Shown in the inbox when the twitch.tv session belongs to someone else. */
export const SESSION_PROBLEM_TEXT = 'Sign in to twitch.tv again to load your whispers';
