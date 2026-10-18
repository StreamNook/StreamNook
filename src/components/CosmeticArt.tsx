// A StreamNook cosmetic's art for pickers and cards. It never shows a broken
// image: art that fails to load (a network that cannot reach the CDN) gives way
// to a faint StreamNook mark in the same box, and Rust's disk copy swaps in on
// its own once it lands. The alt is empty on purpose, since a failed load with
// alt text draws the broken-image icon; the surrounding tile carries the name.
import { useState, useSyncExternalStore, type HTMLAttributes } from 'react';

import streamNookLogo from '../assets/streamnook-logo-128.webp';
import { getStreamnookArtVersion, subscribeStreamnookArt } from '../services/streamnookArt';
import { resolveCosmeticAsset } from './cosmeticAssets';

type ArtCosmetic = Parameters<typeof resolveCosmeticAsset>[0];

export function CosmeticArt({
  cosmetic,
  chatSize = false,
  className,
  loading,
  ...rest
}: {
  cosmetic: ArtCosmetic;
  /** The 64px variant where one exists, for renders at 64 CSS px or smaller. */
  chatSize?: boolean;
  /** Sizing classes, shared by the image and its placeholder. */
  className: string;
  loading?: 'lazy' | 'eager';
} & Omit<HTMLAttributes<HTMLElement>, 'className'>) {
  useSyncExternalStore(subscribeStreamnookArt, getStreamnookArtVersion, getStreamnookArtVersion);
  const src = resolveCosmeticAsset(cosmetic, { chatSize });
  const [failedSrc, setFailedSrc] = useState<string | null>(null);

  if (!src || failedSrc === src) {
    return (
      <span {...rest} className={`${className} inline-flex items-center justify-center`}>
        {/* The mark points right, so its weight sits left of its box; the nudge
            centers it by eye. */}
        <img
          src={streamNookLogo}
          alt=""
          className="h-1/2 w-1/2 translate-x-[6%] object-contain opacity-20 grayscale"
          draggable={false}
        />
      </span>
    );
  }
  return (
    <img
      {...rest}
      src={src}
      alt=""
      className={className}
      loading={loading}
      draggable={false}
      onError={() => setFailedSrc(src)}
    />
  );
}
