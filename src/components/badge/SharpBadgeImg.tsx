// A Twitch badge image that switches to Penrose's sharp copy once Rust has it on
// disk. It draws Twitch's own image first and never puts the CDN URL in `src`: the
// sharp file is fetched by Rust (cache_streamnook_art) and drawn only from the disk
// copy, so a blocked or slow CDN can never leave a tile blank.
import { useEffect, useState, useSyncExternalStore } from 'react';
import type { BadgeArt } from '../../services/badgeStanding';
import {
  getStreamnookArtVersion,
  requestStreamnookArt,
  streamnookArtCopyOf,
  subscribeStreamnookArt,
} from '../../services/streamnookArt';
import { pickSharp } from './sharpPick';

/** Disk copies that failed to load this session, by the exact `src` tried. A
 *  copy in here is never drawn again, so an error can never loop. */
export const _failedSharpCopies = new Set<string>();

/** What the image draws: the disk copy of the picked sharp file when there is one and it has not failed
 *  this session, else Twitch's image. Never the CDN URL itself. */
export function sharpSrc(twitch: string, picked: string | null, copy: string | null, failed: ReadonlySet<string>): string {
  if (!picked || !copy || failed.has(copy)) return twitch;
  return copy;
}

// A server render has no art store to follow; it reads the same version, so the
// first paint matches whatever the client would draw.
const getServerSnapshot = getStreamnookArtVersion;

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
  // Re-renders when a piece of art lands on disk.
  useSyncExternalStore(subscribeStreamnookArt, getStreamnookArtVersion, getServerSnapshot);
  // Bumped when a disk copy fails, so the image goes back to Twitch's.
  const [, setFailures] = useState(0);
  const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1;
  const picked = pickSharp(cssPx, dpr, src, art);
  const copy = picked ? streamnookArtCopyOf(picked) : null;

  useEffect(() => {
    if (picked && !copy) requestStreamnookArt(picked);
  }, [picked, copy]);

  const drawn = sharpSrc(src, picked, copy, _failedSharpCopies);
  const onError =
    drawn === src
      ? undefined
      : () => {
          _failedSharpCopies.add(drawn);
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
      onError={onError}
    />
  );
}
