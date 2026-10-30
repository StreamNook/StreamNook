// Which of a song's links each surface shows. Rust stores a song's links in
// display order, the track's own links first, then search pages
// (`search: true`), then services kept from older lookups (links.rs
// sort_providers). Pure (type imports only), so it is tested without a page.

import type { SongEntry } from '../../utils/songHistory';
import type { SongMatch, SongProvider } from '../../utils/songId';

/** What a song's page shows under its links. */
export interface SongLinks {
  /** "Listen on": the track's own links, then its Shazam page. */
  listen: SongProvider[];
  /** "Search on": search pages, a quieter row of logos. */
  search: SongProvider[];
  /** "All services": the song.link page, only when one is stored. */
  allServices: string | null;
}

export function songLinks(
  song: Pick<SongEntry, 'providers' | 'shazam_url' | 'song_link' | 'links_are_searches'>,
): SongLinks {
  // An entry from before per-link searches marks them all at once.
  const isSearch = (p: SongProvider) => !!p.search || !!song.links_are_searches;
  const listen = song.providers.filter((p) => !isSearch(p));
  if (song.shazam_url) listen.push({ name: 'Shazam', url: song.shazam_url });
  return { listen, search: song.providers.filter(isSearch), allServices: song.song_link || null };
}

/**
 * The links the chat card shows, in order: the track's own links, or, only
 * when it has none, its search pages, then the song.link page when one is
 * kept. `search` says the service links are searches, which the card marks.
 */
export function cardLinks(card: Pick<SongMatch, 'providers' | 'song_link'>): { links: SongProvider[]; search: boolean } {
  const direct = card.providers.filter((p) => !p.search);
  const links = direct.length ? direct : card.providers.filter((p) => p.search);
  const out = [...links];
  if (card.song_link) out.push({ name: 'song.link', url: card.song_link });
  return { links: out, search: direct.length === 0 && links.length > 0 };
}

/**
 * The one link the player's toast offers for a match: the first of the
 * track's own links, else its song.link page, else its Shazam page, else its
 * first search page, which the label names as a search.
 */
export function toastLink(
  song: Pick<SongMatch, 'providers' | 'song_link' | 'shazam_url'>,
): { label: string; url: string } | null {
  const direct = song.providers.find((p) => !p.search && p.url);
  if (direct) return { label: `Open in ${direct.name}`, url: direct.url };
  if (song.song_link) return { label: 'Open', url: song.song_link };
  if (song.shazam_url) return { label: 'Open in Shazam', url: song.shazam_url };
  const search = song.providers.find((p) => p.search && p.url);
  return search ? { label: `Search on ${search.name}`, url: search.url } : null;
}
