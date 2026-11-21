import { describe, expect, it } from 'vitest';
import { parseMessage, type BackendChatMessage } from './twitchChat';

const row = (extra: Partial<BackendChatMessage>): BackendChatMessage => ({
  id: 'pin-1',
  username: 'viewer',
  display_name: 'Viewer',
  user_id: '777',
  timestamp: '1700000000000',
  content: 'hi',
  provider: 'twitch',
  channel: 'fixroom',
  badges: [
    {
      name: 'subscriber',
      version: '12',
      image_url_1x: 'https://b/sub12/1',
      image_url_2x: 'https://b/sub12/2',
      image_url_4x: 'https://b/sub12/4',
      title: '1-Year Subscriber',
    },
    { name: 'unknown', version: '1' },
  ],
  tags: { 'room-id': '424242', id: 'pin-1' },
  segments: [{ type: 'text', content: 'hi' }],
  metadata: { is_action: false, is_mentioned: false, is_first_message: false, is_from_shared_chat: false },
  ...extra,
});

describe('a Twitch row whose badges Rust resolved', () => {
  it('draws the art and title it carries, in all three sizes, without this window knowing the chat', () => {
    const parsed = parseMessage(row({ badges_resolved: true }));
    expect(parsed.badges).toHaveLength(1);
    const [badge] = parsed.badges;
    expect(badge.key).toBe('subscriber/12');
    expect(badge.info).toMatchObject({
      image_url_1x: 'https://b/sub12/1',
      image_url_2x: 'https://b/sub12/2',
      image_url_4x: 'https://b/sub12/4',
      title: '1-Year Subscriber',
    });
    expect(badge.info.localUrl).toBeUndefined();
  });

  it('leaves a row Rust did not resolve to this window`s own badge cache', () => {
    const parsed = parseMessage(row({}));
    expect(parsed.badges.map((b) => b.key)).toEqual(['subscriber/12', 'unknown/1']);
    expect(parsed.badges[0].info).toBeNull();
  });
});
