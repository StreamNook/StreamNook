// @vitest-environment node
// Run with: npm test
//
// A tab strip reads Rust's live rows by each tab's own key, in tab order. A
// server render is the first paint, and zustand serves it the store's initial
// state, so the mocked store starts with its rows.
import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('../stores/tabLiveStore', async () => {
  const { create } = await import('zustand');
  const row = (live: boolean, viewer_count: number | null) => ({ live, viewer_count, category: null, title: null });
  const live = { 'twitch:kathi': row(true, 1200), 'kick:kathi': row(false, null), 'youtube:@Amane': row(true, 300) };
  return { startTabLive: vi.fn(), useTabLiveStore: create(() => ({ live })) };
});

import { useTabLive } from './useTabLive';
import { liveWord, type LiveSource } from '../utils/tabLive';

function Probe({ sources }: { sources: LiveSource[] }) {
  const live = useTabLive('test', sources);
  return <>{live.stats.map((s) => `${s.key}=${liveWord(s) ?? 'unknown'}/${s.count ?? '-'}`).join(' ')}</>;
}

describe('useTabLive', () => {
  it('reads each tab its own row, in tab order, and leaves unanswered tabs unknown', () => {
    const html = renderToStaticMarkup(
      <Probe
        sources={[
          { channel: 'Kathi' },
          { channel: 'kathi', provider: 'kick' },
          { channel: '@Amane', provider: 'youtube' },
          { channel: 'elly' },
        ]}
      />,
    );
    expect(html).toBe('twitch:kathi=Live/1200 kick:kathi=Offline/- youtube:@Amane=Live/300 twitch:elly=unknown/-');
  });
});
