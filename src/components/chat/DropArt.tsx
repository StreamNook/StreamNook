// A reward's picture at a fixed size, with the gift glyph when Twitch sends
// none, so a missing image never changes a row's height. `size` 'fill' takes
// the parent's width (the drawer's reward tiles): square by default, or a
// wider box (`aspect`) with the picture fitted inside it, for the wider tiles
// of a row of two.
import { Gift } from 'phosphor-react';

export function DropArt({
  image,
  size,
  radius,
  aspect,
}: {
  image: string;
  size: number | 'fill';
  radius?: number;
  aspect?: string;
}) {
  const r = radius ?? (size === 'fill' ? 8 : size >= 40 ? 10 : size >= 24 ? 7 : 5);
  const box =
    size === 'fill'
      ? { width: '100%', aspectRatio: aspect ?? '1 / 1', borderRadius: r }
      : { width: size, height: size, borderRadius: r };
  const glyph = size === 'fill' ? 22 : Math.round(size * 0.55);
  return image ? (
    <img
      src={image}
      alt=""
      draggable={false}
      style={box}
      className={`drops-art shrink-0 ${aspect ? 'object-contain' : 'object-cover'}`}
    />
  ) : (
    <span style={box} className="drops-art flex shrink-0 items-center justify-center bg-textPrimary/[0.08]">
      <Gift size={glyph} className="text-accent-on-surface" />
    </span>
  );
}
