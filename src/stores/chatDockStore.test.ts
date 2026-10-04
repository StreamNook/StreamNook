import { describe, expect, it } from 'vitest';
import { dockView, type DockedChat } from './chatDockStore';

const chat = (login: string, provider: DockedChat['provider'] = 'twitch'): DockedChat => ({
  provider,
  login,
  channel_id: '1',
  display_name: login,
  avatar_url: null,
  light_on_new: true,
});
const live = { provider: 'twitch' as const, login: 'xqc' };

describe('dockView', () => {
  it('with nothing docked there is only the live chat', () => {
    expect(dockView([], 'twitch:a', live, false)).toEqual({ held: false, shown: null, others: [] });
  });

  it('shows the chosen docked chat, else the live one', () => {
    const chats = [chat('a'), chat('b')];
    expect(dockView(chats, 'twitch:b', live, false).shown?.login).toBe('b');
    expect(dockView(chats, null, live, false).shown).toBeNull();
    expect(dockView(chats, 'twitch:gone', live, false).shown).toBeNull();
  });

  it('a docked chat that is the watched stream is that stream, not a copy', () => {
    const v = dockView([chat('xqc'), chat('a')], 'twitch:xqc', live, false);
    expect(v.shown).toBeNull();
    expect(v.others.map((c) => c.login)).toEqual(['a']);
  });

  it('the same login on another platform is a different chat', () => {
    const v = dockView([chat('xqc', 'kick')], 'kick:xqc', live, false);
    expect(v.shown?.provider).toBe('kick');
  });

  it('sleeps under MultiNook', () => {
    expect(dockView([chat('a')], 'twitch:a', live, true).held).toBe(false);
  });

  it('with no stream open the chosen or first docked chat shows', () => {
    const chats = [chat('a'), chat('b')];
    expect(dockView(chats, null, null, false).shown?.login).toBe('a');
    expect(dockView(chats, 'twitch:b', null, false).shown?.login).toBe('b');
  });
});
