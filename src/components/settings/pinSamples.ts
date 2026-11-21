// The three sample pins the Pin mentions setting previews (SN-284): a fixed
// set, so the preview draws with the switch off and signed out, through the
// same block a chat draws its real pins with. The rows are Twitch rows with no
// cosmetics to load, and the preview never warms the chatter store.

import type { BackendChatMessage, MessageSegment } from '../../services/twitchChat';
import type { MentionPin } from '../../types';

export function samplePin(
  n: number,
  kind: 'mention' | 'reply' | 'word',
  login: string,
  color: string,
  words: string,
  time: string,
): MentionPin {
  const id = `sample-pin-${n}`;
  const segments: MessageSegment[] = [{ type: 'text', content: words }];
  const row: BackendChatMessage = {
    id,
    username: login,
    display_name: login,
    color,
    user_id: `sample-${n}`,
    timestamp: new Date(0).toISOString(),
    content: words,
    badges: [],
    tags: { id, 'display-name': login, color, 'user-id': `sample-${n}`, 'room-id': 'sample' },
    segments,
    metadata: {
      is_action: false,
      is_mentioned: kind === 'mention',
      is_first_message: false,
      formatted_timestamp: time,
      formatted_timestamp_with_seconds: time,
      is_from_shared_chat: false,
    },
  } as BackendChatMessage;
  return {
    entry: {
      id,
      platform: 'twitch',
      room: 'sample',
      channel_login: 'sample',
      sender_id: `sample-${n}`,
      sender_login: login,
      sender_name: login,
      sender_color: color,
      text: words,
      kind,
      at_ms: 0,
      read: true,
      removed: false,
    },
    row,
    channel_mark: null,
    clock: n === 3 ? 'running' : 'held',
    left_ms: 60_000,
    run: 1,
    rev: n,
    day: false,
  };
}

export const SAMPLE_PINS: readonly MentionPin[] = [
  samplePin(1, 'word', 'nightowl', '#B380FF', 'the giveaway starts soon, stay tuned', '9:41 PM'),
  samplePin(2, 'reply', 'mossy', '#2EB872', '@you yes, it starts at ten', '9:43 PM'),
  samplePin(3, 'mention', 'pixelfox', '#1E90FF', '@you are you still here? the raid is starting', '9:45 PM'),
];
