// A Twitch badge image that switches to Penrose's sharp copy once Rust has it on
// disk. It draws Twitch's own image first and never puts the CDN URL in `src`: the
// sharp file is fetched by Rust (cache_streamnook_art) and drawn only from the disk
// copy, so a blocked or slow CDN can never leave a tile blank.
import { useState, useSyncExternalStore } from 'react';
import type { BadgeArt } from '../../services/badgeStanding';
import {
  forgetStreamnookArtCopy,
  requestStreamnookArt,
  streamnookArtCopyOf,
  subscribeStreamnookArt,
} from '../../services/streamnookArt';
import { pickSharp } from './sharpPick';

/** Disk copies that failed to load this session, by `failedKey` of the `src`
 *  tried. A copy in here is never drawn again, so an error can never loop. */
export const _failedSharpCopies = new Set<string>();

/** The key a failed disk copy is remembered by: the exact `src` that failed. */
export function failedKey(drawn: string): string {
  return drawn;
}

/** What the image draws: the disk copy of the picked sharp file when there is one and it has not failed
 *  this session, else Twitch's image. Never the CDN URL itself. */
export function sharpSrc(twitch: string, picked: string | null, copy: string | null, failed: ReadonlySet<string>): string {
  if (!picked || !copy || failed.has(failedKey(copy))) return twitch;
  return copy;
}

/**
 * The request for the sharp file, handed to the Twitch image's own load and
 * error events, so only an image the browser actually loads asks Rust for art
 * (a lazy tile off screen asks for nothing). None while a disk copy exists,
 * whether it is drawn or failed, or with nothing picked.
 */
export function sharpRequestFor(
  drawn: string,
  src: string,
  picked: string | null,
  copy: string | null,
): (() => void) | undefined {
  return drawn === src && picked && !copy ? () => requestStreamnookArt(picked) : undefined;
}

/**
 * A drawn disk copy failed to decode: it is never drawn again this session,
 * and the first tile to see it fail asks Rust to forget that copy, so the next
 * session downloads the art again.
 */
export function sharpCopyFailed(drawn: string, picked: string): void {
  const key = failedKey(drawn);
  if (_failedSharpCopies.has(key)) return;
  _failedSharpCopies.add(key);
  forgetStreamnookArtCopy(picked);
}

export interface SharpBadgeImgProps {
  /** Twitch's image as the surface draws it today. */
  src: string;
  /** The badge's sharp art; used only when `art.i` is exactly `src`. */
  art?: BadgeArt | null;
  /** The drawn width in CSS px, for picking the file. */
  cssPx: number;
  alt: string;
  className?: string;
  loading?: 'lazy' | 'eager';
  decoding?: 'async' | 'sync' | 'auto';
  draggable?: boolean;
}

export function SharpBadgeImg({ src, art, cssPx, alt, className, loading, decoding, draggable }: SharpBadgeImgProps) {
  // Bumped when a disk copy fails, so the image goes back to Twitch's.
  const [, setFailures] = useState(0);
  const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1;
  const picked = pickSharp(cssPx, dpr, src, art);
  // Re-renders only when this tile's own disk copy changes. A server render
  // reads the same copy, so the first paint matches what the client draws.
  const read = () => (picked ? streamnookArtCopyOf(picked) : null);
  const copy = useSyncExternalStore(subscribeStreamnookArt, read, read);

  const drawn = sharpSrc(src, picked, copy, _failedSharpCopies);
  const ask = sharpRequestFor(drawn, src, picked, copy);
  const onError =
    drawn === src || !picked
      ? ask
      : () => {
          sharpCopyFailed(drawn, picked);
          setFailures((n) => n + 1);
        };

  return (
    <img
      src={drawn}
      alt={alt}
      className={className}
      loading={loading}
      decoding={decoding}
      draggable={draggable}
      onLoad={ask}
      onError={onError}
    />
  );
}
