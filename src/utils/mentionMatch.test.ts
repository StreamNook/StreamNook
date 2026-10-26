import { describe, expect, it } from 'vitest';
import type { ChatUser } from '../stores/chatUserStore';
import { matchingUsers } from './mentionMatch';

const user = (userId: string, username: string, lastSeen: number, displayName = username): ChatUser => ({
  userId,
  username,
  displayName,
  color: '#fff',
  lastSeen,
});

const streamer = user('9', 'kaileymae', 0, 'KaileyMae');
const room = (users: ChatUser[]) => new Map(users.map((u) => [u.userId, u]));
const chatters = room([user('1', 'kappa_fan', 300), user('2', 'kaiju', 200), user('3', 'zed', 100)]);
const names = (rows: ChatUser[]) => rows.map((u) => u.username);

describe('matchingUsers', () => {
  it('lists matching chatters newest first', () => {
    expect(names(matchingUsers(chatters, 'ka', 5, null, {}))).toEqual(['kappa_fan', 'kaiju']);
  });

  it('leads with the streamer before they have chatted', () => {
    expect(names(matchingUsers(chatters, 'KA', 5, streamer, {}))).toEqual(['kaileymae', 'kappa_fan', 'kaiju']);
  });

  it('leads with the streamer on a bare @', () => {
    expect(names(matchingUsers(chatters, '', 5, streamer, {}))[0]).toBe('kaileymae');
  });

  it('leaves the streamer out when they do not match', () => {
    expect(names(matchingUsers(chatters, 'ze', 5, streamer, {}))).toEqual(['zed']);
  });

  it("uses the streamer's own row once they have chatted, listed once", () => {
    const chatted = { ...streamer, color: '#f0f', lastSeen: 50 };
    const rows = matchingUsers(room([user('1', 'kappa_fan', 300), chatted]), 'ka', 5, streamer, {});
    expect(names(rows)).toEqual(['kaileymae', 'kappa_fan']);
    expect(rows[0].color).toBe('#f0f');
  });

  it('keeps the list at its limit with the streamer in it', () => {
    expect(matchingUsers(chatters, 'ka', 2, streamer, {})).toHaveLength(2);
  });

  it('matches a nickname the user gave a chatter', () => {
    expect(names(matchingUsers(chatters, 'buddy', 5, null, { '3': { nickname: 'Buddy' } }))).toEqual(['zed']);
  });
});
