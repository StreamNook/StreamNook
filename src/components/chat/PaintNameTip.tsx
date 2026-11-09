// The card a chatter's name shows on hover when they wear a 7TV paint: the
// paint's name, drawn in the paint itself (SN-264, zoox's mock: "Freedom" in
// Freedom). A chatter with no paint shows no card at all; a StreamNook member
// shows their member card instead.
import type { CSSProperties } from 'react';
import { computePaintStyle } from '../../services/seventvService';

export function PaintNameTip({
  paint,
  color,
  shadowMode,
  still,
}: {
  paint: { name: string } & Record<string, unknown>;
  /** The chatter's own color, which a paint falls back to. */
  color: string;
  shadowMode: Parameters<typeof computePaintStyle>[2];
  /** Motion off: the paint holds still, as it does on the name. */
  still: boolean;
}) {
  const style: CSSProperties = computePaintStyle(paint as never, color, shadowMode, still);
  return (
    <span className="inline-block text-[13px] font-bold leading-tight" style={style} aria-label={`7TV paint: ${paint.name}`}>
      {paint.name}
    </span>
  );
}
