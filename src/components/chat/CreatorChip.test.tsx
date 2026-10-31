import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { CreatorChip } from './CreatorChip';

const web = { role: 'uploader' as const, name: 'Alpha', link: { kind: 'web' as const, url: 'https://7tv.app/users/x', site: '7TV' } };

describe('a creator chip', () => {
  it("says Rust's words for where it goes", () => {
    const html = renderToStaticMarkup(<CreatorChip creator={{ ...web, destination: 'Rust says where' }} />);
    expect(html).toContain('>Rust says where<');
  });

  it('sends a Twitch account to the phone sheet when one is given, else to the app', async () => {
    const { creatorRoute } = await import('./CreatorChip');
    const twitch = { role: 'channel' as const, name: 'pokimane', link: { kind: 'twitch' as const, login: 'pokimane', id: '44445592' } };
    expect(creatorRoute(twitch, true)).toEqual({ kind: 'shell' });
    expect(creatorRoute(twitch, false)).toEqual({ kind: 'app', id: '44445592', login: 'pokimane' });
    expect(creatorRoute({ ...twitch, link: { kind: 'twitch', login: 'pokimane' } }, true)).toEqual({
      kind: 'twitch-web',
      url: 'https://www.twitch.tv/pokimane',
    });
    expect(creatorRoute(web, true)).toEqual({ kind: 'web', url: 'https://7tv.app/users/x' });
  });
});
