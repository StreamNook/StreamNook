import { invoke } from '@tauri-apps/api/core';
import {
  captureStreamSamples,
  AUDIO_GRAPH_SUPPORTED,
  AUDIO_GRAPH_REFUSAL,
} from './audioBoost';
import { getActiveVideo, sourceOfVideo, type PlayerSource } from './activeVideo';
import { Logger } from './logger';
import { beginListening, isListening } from './songListening';
import { publishSongResult } from './songResult';
import { mainChatRoom, useAppStore } from '../stores/AppStore';
import { injectSystemMessage, systemSourceFor, useChatConnectionStore } from '../stores/chatConnectionStore';
import { DEFAULT_SONG_ID } from '../types';

export interface SongProvider {
  name: string;
  url: string;
  /** A search page on the service, not the track's own link. */
  search?: boolean;
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
  | { status: 'error'; message: string }
  /** Cancel was pressed, or the player changed under the listening. */
  | { status: 'canceled' }
  /** A listening already runs in this window. */
  | { status: 'busy' };

/** Says nothing: a listening canceled, or one asked for while one runs. */
export const isQuietResult = (r: RecognizeResult): r is Extract<RecognizeResult, { status: 'canceled' | 'busy' }> =>
  r.status === 'canceled' || r.status === 'busy';

export interface RecognizeOptions {
  /** The chat room the answer goes to (a chat-slice key), which the
   *  listening strip shows in; null names none. */
  room?: string | null;
  /** Listening for another window (its request's id): the state and level
   *  are sent there as it listens. */
  remote?: { window: string; id: string } | null;
}

/** What the main player shows (VideoPlayer, the phone's player), for where
 *  Your songs says a song was heard. */
function mainPlayerSource(): PlayerSource | null {
  const app = useAppStore.getState();
  const stream = app.currentStream;
  if (!stream?.user_login) return null;
  const provider = stream.provider ?? 'twitch';
  // An offline room plays the channel's latest broadcast, a VOD.
  const media =
    app.currentMediaType === 'clip'
      ? 'clip'
      : app.currentMediaType === 'video' || app.currentMediaType === 'offline_chat' || app.liveRewind
        ? 'video'
        : 'live';
  // Live, how long the broadcast has run (a VOD's place comes from its player).
  const started = media === 'live' ? Date.parse(stream.started_at) : NaN;
  return {
    provider,
    // A YouTube row's login can be one broadcast's video id; the channel's own
    // id (UC...) stays the same.
    channel_login: provider === 'youtube' && stream.user_id?.startsWith('UC') ? stream.user_id : stream.user_login,
    channel_name: stream.user_name || null,
    media,
    title: stream.title?.trim() || null,
    category: stream.game_name?.trim() || null,
    avatar: stream.profile_image_url || null,
    position_secs: Number.isFinite(started) ? Math.max(0, Math.floor((Date.now() - started) / 1000)) : null,
  };
}

/** `source` with how far into its video `el` was, for a VOD heard (a
 *  live source already carries its run time, and a clip has no place). */
function withPosition(source: PlayerSource | null, el: HTMLMediaElement): PlayerSource | null {
  if (source?.media !== 'video' || !Number.isFinite(el.currentTime)) return source;
  return { ...source, position_secs: Math.floor(el.currentTime) };
}

// Capture from the active player and identify the song. `video` defaults to the
// registered active element, so the chat command can call it with no argument.
// One listening runs per window (songListening.ts holds it, for the strip and
// the note): a second while one runs answers busy and says nothing.
export async function recognizeNowPlaying(
  video?: HTMLMediaElement | null,
  opts: RecognizeOptions = {},
): Promise<RecognizeResult> {
  if (isListening()) return { status: 'busy' };
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
  // Where it is playing, read once (as the listening starts): every attempt
  // listens to the same player.
  const source = withPosition(sourceOfVideo(target, mainPlayerSource), target);

  const listen = beginListening({
    room: opts.room ?? null,
    attempts,
    captureMs: captureSeconds * 1000,
    remote: opts.remote ?? null,
  });
  if (!listen) return { status: 'busy' };
  const { signal } = listen;
  // A player rebuilt or switched to another channel under the listening ends
  // it as canceled: what it heard is no longer that stream.
  const stop = () => listen.abort();
  target.addEventListener('emptied', stop);
  target.addEventListener('pause', stop);

  try {
    // Each attempt listens again (the next window of audio), so a missed window
    // (ad break, quiet moment) gets another shot. A real match returns
    // immediately; only a clean "nothing found" falls through to the next attempt.
    let lastError: string | null = null;
    for (let attempt = 0; attempt < attempts; attempt++) {
      if (attempt > 0) listen.update({ phase: 'listening', attempt: attempt + 1, startedAt: null });
      let pcm: Int16Array | null;
      try {
        pcm = await captureStreamSamples(target, captureSeconds, {
          signal,
          onLevel: listen.level,
          onStart: () => listen.update({ startedAt: Date.now() }),
        });
      } catch (e) {
        if (signal.aborted) return { status: 'canceled' };
        Logger.warn('[SongId] capture failed:', e);
        lastError = 'Could not capture the stream audio.';
        continue;
      }
      if (signal.aborted) return { status: 'canceled' };
      if (!pcm || pcm.length < 16000) {
        lastError = 'Could not capture enough audio.';
        continue;
      }

      listen.update({ phase: 'finding' });
      if (signal.aborted) return { status: 'canceled' };
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
  } finally {
    target.removeEventListener('emptied', stop);
    target.removeEventListener('pause', stop);
    // Only now, with the capture settled: a listening started meanwhile would
    // have overlapped it.
    listen.end();
  }
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
  // A room this page does not hold that is no longer the one the main view
  // holds (the viewer switched channels while it listened): the answer
  // belongs to a chat that is gone, so it is not posted anywhere else.
  if (room && room.toLowerCase() !== mainChatRoom()?.toLowerCase()) return;
  // No room (a stream with no chat of its own), or the main view's room held
  // elsewhere (its chat lives in a popout): the page's one open chat, as before.
  window.dispatchEvent(
    new CustomEvent('twitch-system-message', { detail: { message: text, songCard } }),
  );
}

export function announceSong(result: RecognizeResult, room: string | null = null): void {
  if (isQuietResult(result)) return;
  // The answer also shows for a moment over that chat's composer, where the
  // listening strip was.
  publishSongResult(result, room);
  if (result.status === 'match') {
    emitSongMessage(room, `♪ ${result.song.title} by ${result.song.artist}`, result.song);
  } else if (result.status === 'no-match') {
    emitSongMessage(room, "Couldn't identify the song. Try again when the music is clearer.");
  } else {
    emitSongMessage(room, `Song ID: ${result.message}`);
  }
}
