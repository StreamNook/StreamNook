// The glyphs the phone's menus share for favorites and subscribing. Favorite
// is a gold star (Follow already wears the heart); Subscribe always wears the
// channel's badge art from Rust and keeps an empty slot while it loads, so the
// row never shows a stand-in and never shifts when the art lands.
import React from 'react';
import { Star } from 'phosphor-react';

export function favoriteLabel(on: boolean): string {
  return on ? 'Unfavorite' : 'Favorite';
}

export const FavoriteStar: React.FC<{ on: boolean; size: number }> = ({ on, size }) => (
  <Star
    size={size}
    weight={on ? 'fill' : 'regular'}
    className={on ? 'text-yellow-400' : 'text-textSecondary'}
    aria-hidden
  />
);

export const SubscribeBadge: React.FC<{ url: string | null; size: number }> = ({ url, size }) =>
  url ? (
    <img src={url} alt="" draggable={false} style={{ width: size, height: size }} />
  ) : (
    <span aria-hidden className="block" style={{ width: size, height: size }} />
  );
