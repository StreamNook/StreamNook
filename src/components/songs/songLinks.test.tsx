// A song's links, as each surface shows them: the track's own links first
// everywhere; the chat card falls back to search pages only when it has none;
// the player's toast names a search page as one; the song's page splits them
// into "Listen on" and a quieter "Search on", with "All services" only when a
// song.link page is stored.
import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('../../stores/AppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { cardLinks, songLinks, toastLink } from './songLinks';
import { SongCard } from '../chat/SongCard';
import type { SongMatch } from '../../utils/songId';

const direct = [
  { name: 'Apple Music', url: 'https://music.apple.com/us/song/1' },
  { name: 'Deezer', url: 'https://www.deezer.com/track/2' },
];
const search = [
  { name: 'Spotify', url: 'https://open.spotify.com/search/sade', search: true },
  { name: 'Tidal', url: 'https://tidal.com/search?q=sade', search: true },
];
const match = (over: Partial<SongMatch> = {}): SongMatch => ({
  title: 'Kiss of Life',
  artist: 'Sade',
  album: 'Love Deluxe',
  album_art: null,
  shazam_url: null,
  song_link: null,
  providers: [...direct, ...search],
  key: null,
  ...over,
});

describe('the chat card', () => {
  it('shows the track’s own links, never its searches beside them', () => {
    expect(cardLinks(match())).toEqual({ links: direct, search: false });
  });

  it('falls back to search pages only when it has no link of its own, marked as searches', () => {
    expect(cardLinks(match({ providers: search }))).toEqual({ links: search, search: true });
    expect(cardLinks(match({ providers: [] }))).toEqual({ links: [], search: false });
  });

  it('keeps a stored song.link page last', () => {
    const { links } = cardLinks(match({ song_link: 'https://song.link/s/1' }));
    expect(links.map((l) => l.name)).toEqual(['Apple Music', 'Deezer', 'song.link']);
  });

  it('draws them so', () => {
    const own = renderToStaticMarkup(<SongCard card={match()} />);
    expect(own).toContain('Apple Music');
    expect(own).not.toContain('Spotify');
    expect(own).not.toContain('Search on');
    const searches = renderToStaticMarkup(<SongCard card={match({ providers: search })} />);
    expect(searches).toContain('Search on');
    expect(searches).toContain('Spotify');
  });
});

describe("the player's toast", () => {
  it('opens the first of the track’s own links', () => {
    expect(toastLink(match({ providers: [...search, ...direct] }))).toEqual({
      label: 'Open in Apple Music',
      url: 'https://music.apple.com/us/song/1',
    });
  });

  it('else its song.link page, its Shazam page, then a search named as one', () => {
    expect(toastLink(match({ providers: search, song_link: 'https://song.link/s/1' }))?.label).toBe('Open');
    expect(toastLink(match({ providers: search, shazam_url: 'https://www.shazam.com/track/1' }))?.label).toBe('Open in Shazam');
    expect(toastLink(match({ providers: search }))).toEqual({
      label: 'Search on Spotify',
      url: 'https://open.spotify.com/search/sade',
    });
    expect(toastLink(match({ providers: [] }))).toBeNull();
  });
});

describe("the song's page", () => {
  const entry = { providers: [...direct, ...search], shazam_url: null, song_link: null } as const;

  it('"Listen on" holds the track’s own links then Shazam; "Search on" the searches', () => {
    const links = songLinks({ ...entry, providers: [...entry.providers], shazam_url: 'https://www.shazam.com/track/9' });
    expect(links.listen.map((l) => l.name)).toEqual(['Apple Music', 'Deezer', 'Shazam']);
    expect(links.search.map((l) => l.name)).toEqual(['Spotify', 'Tidal']);
  });

  it('"All services" only when a song.link page is stored', () => {
    expect(songLinks({ ...entry, providers: [...entry.providers] }).allServices).toBeNull();
    expect(songLinks({ ...entry, providers: [...entry.providers], song_link: 'https://song.link/s/1' }).allServices).toBe(
      'https://song.link/s/1',
    );
  });

  it('an entry from before per-link searches counts every link as one', () => {
    const links = songLinks({ ...entry, providers: [...direct], links_are_searches: true });
    expect(links.listen).toEqual([]);
    expect(links.search.map((l) => l.name)).toEqual(['Apple Music', 'Deezer']);
  });
});
