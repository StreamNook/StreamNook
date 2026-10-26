// @vitest-environment node
// Run with: npm test
//
// The builder's live preview reads each source's chat slice by the key the
// chat store stores it under. A YouTube channel id is mixed case and the store
// folds it, so a case-keeping lookup missed the slice and a connected source
// sat on "Connecting to …" with its messages never shown.

import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

vi.mock('../../stores/AppStore', () => ({
  useAppStore: Object.assign((select: (s: object) => unknown) => select({}), { getState: () => ({}) }),
}));

const store = vi.hoisted(() => ({ channels: new Map<string, unknown>() }));
vi.mock('../../stores/chatConnectionStore', () => ({
  useChatConnectionStore: Object.assign((select: (s: { revision: number }) => unknown) => select({ revision: 0 }), {
    getState: () => store,
  }),
  acquireChannel: vi.fn(() => Promise.resolve()),
  releaseChannel: vi.fn(() => Promise.resolve()),
}));

import { LiveOverlayFeed } from './LiveOverlayFeed';
import { DEFAULT_OVERLAY_STYLE } from './overlayConfig';

const CHANNEL_ID = 'UCx7YtSqF9Hi1EbDmmlcpXcQ';

const row = (id: string, content: string) => ({
  id,
  user_id: 'UCchatter',
  username: 'harborlight',
  display_name: 'Harborlight',
  color: '#6aa1ff',
  badges: [],
  timestamp: new Date().toISOString(),
  content,
  provider: 'youtube',
  channel: CHANNEL_ID,
  emotes: [],
  tags: {},
});

const render = () =>
  renderToStaticMarkup(
    <LiveOverlayFeed sources={[{ provider: 'youtube', channel: CHANNEL_ID }]} style={DEFAULT_OVERLAY_STYLE} />,
  );

describe('live preview slice keys', () => {
  it('shows a connected YouTube source stored under its folded key', () => {
    store.channels = new Map([
      [`youtube:${CHANNEL_ID.toLowerCase()}`, { isConnected: true, messages: [row('m1', 'first from youtube')] }],
    ]);
    const html = render();
    expect(html).toContain('first from youtube');
    expect(html).not.toContain('Connecting to');
  });

  it('says connected, not connecting, before the first YouTube message', () => {
    store.channels = new Map([[`youtube:${CHANNEL_ID.toLowerCase()}`, { isConnected: true, messages: [] }]]);
    expect(render()).toContain('Connected');
  });
});
