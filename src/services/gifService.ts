// Twitch chat GIFs: the page's thin view of the Rust GIF commands.
//
// Everything of consequence happens in Rust (src-tauri/src/commands/gifs.rs):
// eligibility comes from Twitch's private `gifPickerConfig`, the GIPHY search
// key it returns NEVER crosses IPC, and sending goes through Twitch's own
// `sendGifMessage` mutation because neither Helix nor IRC can post a GIF.
// This file only types the boundary.

import { invoke } from '@tauri-apps/api/core';

export interface GifPickerStatus {
  /** The broadcaster allows GIFs in this channel. */
  is_enabled: boolean;
  /** This account may send them here (Twitch's server-side Tier 2/3 gate). */
  is_allowlisted: boolean;
  /** Enabled + allowlisted + a usable search key: the tab can do its job. */
  can_use: boolean;
  content_rating: string | null;
}

export interface GifItem {
  id: string;
  title: string;
  /** Small looping preview for the grid, not the full asset. */
  preview_url: string;
  /** Full asset URL. Sent to Twitch verbatim; never rewrite it. */
  url: string;
  width: number;
  height: number;
}

export interface SendGifOutcome {
  sent: boolean;
  /** Twitch's own error enum, e.g. TEMPORARILY_UNAVAILABLE. */
  error: string | null;
  /** Cooldown in seconds. Honour it; never auto-retry. */
  seconds_until_can_send: number;
}

export function getGifPickerStatus(channelId: string): Promise<GifPickerStatus> {
  return invoke<GifPickerStatus>('get_gif_picker_status', { channelId });
}

export function searchGifs(channelId: string, query: string, offset = 0): Promise<GifItem[]> {
  return invoke<GifItem[]>('search_gifs', { channelId, query: query || null, offset });
}

export function sendGifMessage(
  channelId: string,
  gif: GifItem,
  searchTerm: string,
): Promise<SendGifOutcome> {
  return invoke<SendGifOutcome>('send_gif_message', {
    channelId,
    gifId: gif.id,
    gifUrl: gif.url,
    searchTerm: searchTerm || null,
  });
}

/** Plain-language reason a send was refused, for the picker's inline notice. */
export function gifSendMessage(outcome: SendGifOutcome): string {
  if (outcome.sent) return '';
  const wait = outcome.seconds_until_can_send;
  if (outcome.error === 'TEMPORARILY_UNAVAILABLE') {
    return 'GIFs are temporarily unavailable. Try again in a moment.';
  }
  if (wait > 0) return `Slow down a moment, you can send another GIF in ${wait}s.`;
  return 'That GIF could not be sent.';
}
