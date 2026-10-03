import { describe, expect, it } from 'vitest';
import { pinPair, type ChatPin } from './chatPinStore';

const pin: ChatPin = { provider: 'twitch', login: 'rainyyay', channel_id: '1', display_name: 'Rainyyay', avatar_url: null };

describe('pinPair', () => {
  it('without a pin there is only the live chat', () => {
    expect(pinPair(null, 'pinned', { provider: 'twitch', login: 'xqc' }, false)).toEqual({
      split: false,
      showPinned: false,
      held: false,
    });
  });

  it('a pin on another channel splits, and the chosen side shows', () => {
    const live = { provider: 'twitch' as const, login: 'xqc' };
    expect(pinPair(pin, 'pinned', live, false)).toEqual({ split: true, showPinned: true, held: true });
    expect(pinPair(pin, 'live', live, false)).toEqual({ split: true, showPinned: false, held: true });
  });

  it('pinning the stream you are watching is one chat, whatever the side', () => {
    const live = { provider: 'twitch' as const, login: 'RainyYay' };
    expect(pinPair(pin, 'pinned', live, false)).toEqual({ split: false, showPinned: false, held: true });
  });

  it('the same login on another platform is a different chat', () => {
    const live = { provider: 'kick' as const, login: 'rainyyay' };
    expect(pinPair(pin, 'pinned', live, false).split).toBe(true);
  });

  it('sleeps under MultiNook', () => {
    expect(pinPair(pin, 'pinned', { provider: 'twitch', login: 'xqc' }, true)).toEqual({
      split: false,
      showPinned: false,
      held: false,
    });
  });

  it('with no stream open the pinned chat is the only one', () => {
    expect(pinPair(pin, 'live', null, false)).toEqual({ split: false, showPinned: true, held: true });
  });
});
