import { invoke } from '@tauri-apps/api/core';
import {
  captureStreamSamples,
  AUDIO_GRAPH_SUPPORTED,
  AUDIO_GRAPH_REFUSAL,
} from './audioBoost';
import { getActiveVideo, sourceOfVideo, type PlayerSource } from './activeVideo';
import { Logger } from './logger';
import { useAppStore } from '../stores/AppStore';
import { injectSystemMessage, systemSourceFor, useChatConnectionStore } from '../stores/chatConnectionStore';
import { DEFAULT_SONG_ID } from '../types';

export interface SongProvider {
  name: string;
  url: string;
}

export interface SongMatch {
  title: string;
  artist: string;
  album: string | null;
  album_art: string | null;
  shazam_url: string | null;
  song_link: string | null;
  providers: SongProvider[];
  /** Its key in Your songs once Rust filed it there; null when it could not
   *  be kept. */
  key?: string | null;
}

// Encode little-endian i16 PCM as base64 for the Tauri bridge (smaller and
// faster than passing a giant JSON number array).
function pcmToBase64(pcm: Int16Array): string {
  const bytes = new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength);
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

export type RecognizeResult =
  | { status: 'match'; song: SongMatch }
  | { status: 'no-match' }
  | { status: 'error'; message: string };

/** What the main player shows (VideoPlayer, the phone's player), for Your
 *  songs' "heard on" line. */
function mainPlayerSource(): PlayerSource | null {
  const app = useAppStore.getState();
  const stream = app.currentStream;
  if (!stream?.user_login) return null;
  const provider = stream.provider ?? 'twitch';
  const media =
    app.currentMediaType === 'clip' ? 'clip' : app.currentMediaType === 'video' || app.liveRewind ? 'video' : 'live';
  return {
    provider,
    // A YouTube row's login can be one broadcast's video id; the channel's own
    // id (UC...) stays the same.
    channel_login: provider === 'youtube' && stream.user_id?.startsWith('UC') ? stream.user_id : stream.user_login,
    channel_name: stream.user_name || null,
    media,
  };
}

// Capture from the active player and identify the song. `video` defaults to the
// registered active element, so the chat command can call it with no argument.
export async function recognizeNowPlaying(
  video?: HTMLMediaElement | null,
): Promise<RecognizeResult> {
  const target = video ?? getActiveVideo();
  if (!target) {
    return { status: 'error', message: 'No stream is playing.' };
  }
  // Recognition listens through the same Web Audio tap Audio Boost uses, so it
  // is closed wherever that tap is. Answering with the real reason beats
  // "could not capture the stream audio", which reads like a transient failure
  // worth retrying.
  if (!AUDIO_GRAPH_SUPPORTED) {
    return { status: 'error', message: AUDIO_GRAPH_REFUSAL };
  }
  if (target.paused || target.muted || target.readyState < 2) {
    // Muted captures silence; paused has nothing to capture.
    return { status: 'error', message: 'Play the stream (unmuted) first.' };
  }

  const cfg = {
    ...DEFAULT_SONG_ID,
    ...(useAppStore.getState().settings.video_player?.song_id ?? {}),
  };
  const captureSeconds = Math.min(30, Math.max(3, cfg.capture_seconds || DEFAULT_SONG_ID.capture_seconds));
  const attempts = 1 + Math.min(3, Math.max(0, cfg.retries ?? DEFAULT_SONG_ID.retries));
  // Where it is playing, read once: every attempt listens to the same player.
  const source = sourceOfVideo(target, mainPlayerSource);

  // Each attempt listens again (the next window of audio), so a missed window
  // (ad break, quiet moment) gets another shot. A real match returns
  // immediately; only a clean "nothing found" falls through to the next attempt.
  let lastError: string | null = null;
  for (let attempt = 0; attempt < attempts; attempt++) {
    let pcm: Int16Array | null;
    try {
      pcm = await captureStreamSamples(target, captureSeconds);
    } catch (e) {
      Logger.warn('[SongId] capture failed:', e);
      lastError = 'Could not capture the stream audio.';
      continue;
    }
    if (!pcm || pcm.length < 16000) {
      lastError = 'Could not capture enough audio.';
      continue;
    }

    try {
      const song = await invoke<SongMatch | null>('identify_song', {
        audioB64: pcmToBase64(pcm),
        source,
      });
      if (song && song.title) return { status: 'match', song };
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      Logger.warn('[SongId] identify failed:', message);
      // A hard error (rate limit, network) won't improve on retry.
      return { status: 'error', message };
    }
  }

  return lastError ? { status: 'error', message: lastError } : { status: 'no-match' };
}

// Drop the result into chat as a local system message, in `room` (the chat the
// command was typed in, or the room the main view holds for the playing
// stream) when this page holds that room. A match also carries the structured
// card (album art + clickable service links); misses and errors are plain text.
function emitSongMessage(room: string | null, text: string, songCard?: SongMatch): void {
  // Chat slices are stored under the lowercased key whatever case a room key
  // arrives in (a YouTube channel id keeps its case in makeKey), so the lookup
  // folds the same way; injectSystemMessage folds on its own.
  if (room && useChatConnectionStore.getState().channels.has(room.toLowerCase())) {
    injectSystemMessage(room, text, songCard, systemSourceFor(room));
    return;
  }
  // No room, or one this page does not hold (its chat lives in a popout): the
  // page's one open chat, as before.
  window.dispatchEvent(
    new CustomEvent('twitch-system-message', { detail: { message: text, songCard } }),
  );
}

export function announceSong(result: RecognizeResult, room: string | null = null): void {
  if (result.status === 'match') {
    emitSongMessage(room, `♪ ${result.song.title} by ${result.song.artist}`, result.song);
  } else if (result.status === 'no-match') {
    emitSongMessage(room, "Couldn't identify the song. Try again when the music is clearer.");
  } else {
    emitSongMessage(room, `Song ID: ${result.message}`);
  }
}
