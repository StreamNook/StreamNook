// Argument resolution for /pin.
//
// Three shapes, matching how people actually reach for it:
//   /pin <message id> [duration]   pin a specific message
//   /pin @user | user [duration]   pin that person's most recent message
//   /pin <text>                    send <text> as your message and pin it
//
// A name only means "their latest message" when nothing but an optional
// duration follows it; "/pin @bob nice play" sends that text. A bare word that
// is nobody's recent message is text too. The send form takes no duration,
// because a trailing "5m" in a sentence ("back in 5m") is part of the message.

import { useChatConnectionStore } from '../stores/chatConnectionStore';
import { sliceLookupKey } from './providerKey';

export type PinTarget =
  | {
      kind: 'message';
      messageId: string;
      /** null = pinned until the stream ends, which is Twitch's default. */
      durationSeconds: number | null;
    }
  | { kind: 'send'; text: string };

const UNITS: Record<string, number> = { s: 1, m: 60, h: 3600 };

/** "60", "60s", "5m", "1h" -> seconds. null when it isn't a duration at all. */
function parseDuration(token: string | undefined): number | null {
  if (!token) return null;
  const m = token.match(/^(\d+)([smh])?$/);
  if (!m) return null;
  const n = Number(m[1]);
  if (!Number.isFinite(n) || n <= 0) return null;
  return n * (m[2] ? UNITS[m[2]] : 1);
}

/** Twitch message ids are UUIDs; anything else is a name or free text. */
function looksLikeMessageId(token: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(token);
}

/** Most recent message from a login in the local buffer, newest first. */
function latestMessageIdFrom(channel: string, login: string): string | null {
  const slice = useChatConnectionStore.getState().channels.get(sliceLookupKey('twitch', channel));
  if (!slice) return null;
  const wanted = login.toLowerCase();
  for (let i = slice.messages.length - 1; i >= 0; i--) {
    const m = slice.messages[i];
    if (!m) continue;
    // Raw IRC rows (some backfill) carry the login in the prefix and the id in
    // the tags; structured rows carry both as fields.
    const login =
      typeof m === 'string' ? m.match(/ :([^!\s]+)![^ ]+ PRIVMSG /)?.[1] : m.username;
    const id = typeof m === 'string' ? m.match(/(?:^@|;)id=([^;\s]+)/)?.[1] : m.id;
    // An own message still waiting for its echo has a local id Twitch never issued.
    if (!id || !login || id.startsWith('local-')) continue;
    if (login.toLowerCase() === wanted) return id;
  }
  return null;
}

/** Resolve /pin arguments. `latestFrom` answers "the newest message id from
 *  this login in this chat", or null when they have none. */
export function resolvePinArgs(
  args: string[],
  latestFrom: (login: string) => string | null,
): PinTarget | { error: string } {
  const tokens = args.filter(Boolean);
  if (tokens.length === 0) {
    return { error: '/pin needs a message id, a username or a message to send' };
  }

  const [first, ...rest] = tokens;

  if (looksLikeMessageId(first)) {
    return { kind: 'message', messageId: first, durationSeconds: parseDuration(rest[0]) };
  }

  const nameOnly = rest.length === 0 || (rest.length === 1 && parseDuration(rest[0]) !== null);
  if (nameOnly) {
    const name = first.replace(/^@/, '');
    const found = name ? latestFrom(name) : null;
    if (found) {
      return { kind: 'message', messageId: found, durationSeconds: parseDuration(rest[0]) };
    }
    if (first.startsWith('@')) {
      return { error: `No recent message from ${name || first} in this chat` };
    }
  }

  return { kind: 'send', text: tokens.join(' ') };
}

export function resolvePinTarget(
  channel: string,
  args: string[],
): PinTarget | { error: string } {
  return resolvePinArgs(args, (login) => latestMessageIdFrom(channel, login));
}
