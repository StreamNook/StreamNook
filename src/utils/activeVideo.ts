// Module-level handle to the currently active player <video> element. The chat
// "/song" command runs outside the player component, so it needs a way to reach
// the element that's actually producing audio. VideoPlayer registers its element
// here; the getter falls back to scanning the DOM for an audible video so the
// MultiNook tiles (which don't go through VideoPlayer) still work.

import type { ProviderId } from '../types/providers';

let activeVideo: HTMLVideoElement | null = null;

export function setActiveVideo(el: HTMLVideoElement | null): void {
  activeVideo = el;
}

export function getActiveVideo(): HTMLVideoElement | null {
  if (activeVideo && activeVideo.isConnected && !activeVideo.paused) return activeVideo;

  const videos = Array.from(document.querySelectorAll('video'));
  // Prefer a tile that's actually audible (playing and unmuted) for MultiNook.
  const audible = videos.find((v) => !v.paused && !v.muted && v.volume > 0);
  if (audible) return audible;

  return (activeVideo && activeVideo.isConnected ? activeVideo : null) ?? videos[0] ?? null;
}

/** The channel a player shows, as Your songs records where a song was heard
 *  (the `source` of `identify_song`), and the stream, video or clip it
 *  shows. The fields after `media` are missing from songs kept before them. */
export interface PlayerSource {
  provider: ProviderId;
  channel_login: string;
  channel_name: string | null;
  media: 'live' | 'video' | 'clip' | null;
  /** The stream's, video's or clip's title. */
  title?: string | null;
  /** Its category ("Just Chatting"). */
  category?: string | null;
  /** The channel's picture. */
  avatar?: string | null;
  /** Seconds in: the broadcast's run time live, the video's own time on a VOD. */
  position_secs?: number | null;
}

// What each player other than the main one (a MultiNook tile) says it shows.
const sources = new WeakMap<HTMLMediaElement, PlayerSource>();

export function setVideoSource(el: HTMLMediaElement, source: PlayerSource | null): void {
  if (source) sources.set(el, source);
  else sources.delete(el);
}

/** The channel `el` is playing: what its player registered, else `main()` when
 *  `el` is the main player's own element (VideoPlayer and the phone's player
 *  register it with setActiveVideo), else null. Never guesses: a tile's song
 *  is never put on the main stream. */
export function sourceOfVideo(el: HTMLMediaElement, main: () => PlayerSource | null): PlayerSource | null {
  return sources.get(el) ?? (el === activeVideo ? main() : null);
}
