import { beforeEach, describe, expect, it, vi } from 'vitest';

// What the main player tells Rust about where a song was heard.
const app: { current: Record<string, unknown> } = { current: {} };
const invoke = vi.fn();
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => app.current }, mainChatRoom: () => null }));
vi.mock('../stores/chatConnectionStore', () => ({
  injectSystemMessage: vi.fn(),
  systemSourceFor: () => undefined,
  useChatConnectionStore: { getState: () => ({ channels: new Map() }) },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('./audioBoost', () => ({
  AUDIO_GRAPH_SUPPORTED: true,
  AUDIO_GRAPH_REFUSAL: '',
  captureStreamSamples: () => Promise.resolve(new Int16Array(20000)),
}));

import { recognizeNowPlaying } from './songId';
import { setActiveVideo } from './activeVideo';

const stream = {
  user_login: 'xqc',
  user_name: 'xQc',
  user_id: '1',
  title: '  react andy ',
  game_name: 'Just Chatting',
  profile_image_url: 'https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-300x300.png',
  started_at: '2026-10-28T10:00:00Z',
};

function player(currentTime: number): HTMLVideoElement {
  const el = {
    paused: false,
    muted: false,
    readyState: 4,
    currentTime,
    // The listening ends as canceled if the player is rebuilt or paused under it.
    addEventListener: () => {},
    removeEventListener: () => {},
  } as unknown as HTMLVideoElement;
  setActiveVideo(el);
  return el;
}

async function sourceSent(el: HTMLVideoElement) {
  await recognizeNowPlaying(el);
  return (invoke.mock.calls[0][1] as { source: Record<string, unknown> | null }).source;
}

describe('where a song was heard, as the main player says it', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({ title: 'One', artist: 'A' });
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2026-10-28T12:14:30Z'));
  });

  it('names the stream live, with how long the broadcast has run', async () => {
    app.current = { settings: {}, currentStream: stream, currentMediaType: 'live', liveRewind: null };
    expect(await sourceSent(player(12))).toEqual({
      provider: 'twitch',
      channel_login: 'xqc',
      channel_name: 'xQc',
      media: 'live',
      title: 'react andy',
      category: 'Just Chatting',
      avatar: stream.profile_image_url,
      position_secs: 2 * 3600 + 14 * 60 + 30,
    });
  });

  it("takes a VOD's place from its player", async () => {
    app.current = { settings: {}, currentStream: stream, currentMediaType: 'video', liveRewind: null };
    expect(await sourceSent(player(4321.6))).toMatchObject({ media: 'video', position_secs: 4321 });
  });

  it("calls an offline room's broadcast a VOD", async () => {
    app.current = {
      settings: {},
      currentStream: { ...stream, started_at: '2026-10-20T00:00:00Z' },
      currentMediaType: 'offline_chat',
      liveRewind: null,
    };
    expect(await sourceSent(player(90))).toMatchObject({ media: 'video', position_secs: 90 });
  });

  it('gives a clip no place, and leaves out what the stream lacks', async () => {
    app.current = {
      settings: {},
      currentStream: { ...stream, title: '', game_name: '', profile_image_url: '' },
      currentMediaType: 'clip',
      liveRewind: null,
    };
    expect(await sourceSent(player(5))).toMatchObject({
      media: 'clip',
      title: null,
      category: null,
      avatar: null,
      position_secs: null,
    });
  });
});
