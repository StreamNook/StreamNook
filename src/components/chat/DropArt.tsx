// A reward's picture at a fixed size, with the gift glyph when Twitch sends
// none, so a missing image never changes a row's height.
import { Gift } from 'phosphor-react';

export function DropArt({ image, size }: { image: string; size: 18 | 26 }) {
  const box = { width: size, height: size };
  return image ? (
    <img
      src={image}
      alt=""
      draggable={false}
      style={box}
      className="shrink-0 rounded-[5px] object-cover ring-1 ring-white/10"
    />
  ) : (
    <span style={box} className="flex shrink-0 items-center justify-center rounded-[5px] bg-textPrimary/[0.08]">
      <Gift size={size === 18 ? 11 : 15} className="text-accent-on-surface" />
    </span>
  );
}
