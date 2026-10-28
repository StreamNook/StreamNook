// The logo a song's link is drawn with, by the service name Rust gives the
// link. Rust names a link only when it goes to that service's own host
// (src-tauri/src/services/song_id/services.rs); serviceMarks.test.ts holds
// this table to that one, so every name Rust can send has a mark here. Values
// are SocialLogo's marks. Deezer and Amazon Music have no mark that reads at
// this size, so they take the plain music note.
export const SERVICE_MARK: Readonly<Record<string, string>> = {
  Spotify: 'spotify',
  'Apple Music': 'applemusic',
  'YouTube Music': 'youtubemusic',
  YouTube: 'youtube',
  SoundCloud: 'soundcloud',
  Tidal: 'tidal',
  Deezer: 'music',
  'Amazon Music': 'music',
  Pandora: 'pandora',
  Shazam: 'shazam',
  'song.link': 'links',
};

/** The mark for a link Rust named `name`. */
export const markOf = (name: string): string => SERVICE_MARK[name] ?? 'music';
