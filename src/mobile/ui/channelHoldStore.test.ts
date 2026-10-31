import { beforeEach, describe, expect, it } from 'vitest';
import type { TwitchStream } from '../../types';
import { useChannelHoldSheet } from './channelHoldStore';

const stream = (login: string) => ({ id: login, user_id: '1', user_login: login, user_name: login }) as TwitchStream;

describe('channel hold sheet store', () => {
  beforeEach(() => useChannelHoldSheet.setState({ held: null }));

  it('holds the card and its tap until closed, whatever happens to the card', () => {
    const taps: string[] = [];
    const onPress = (s: TwitchStream) => taps.push(s.user_login);
    useChannelHoldSheet.getState().open({ stream: stream('alpha'), onPress });
    // Nothing the card does reaches the store: the held entry is the store's own.
    const held = useChannelHoldSheet.getState().held;
    expect(held?.stream.user_login).toBe('alpha');
    held?.onPress(held.stream);
    expect(taps).toEqual(['alpha']);
    useChannelHoldSheet.getState().close();
    expect(useChannelHoldSheet.getState().held).toBeNull();
  });

  it('a second hold replaces the first', () => {
    const onPress = () => {};
    useChannelHoldSheet.getState().open({ stream: stream('alpha'), onPress });
    useChannelHoldSheet.getState().open({ stream: stream('beta'), onPress });
    expect(useChannelHoldSheet.getState().held?.stream.user_login).toBe('beta');
  });
});
