import type { CSSProperties } from 'react';
import type { HypeTrainLookData } from '../../types';
import { badgeLook } from '../../utils/hypeTrainLook';
import { CardChip } from './CardChip';

export interface HypeTrainBadgeInfo {
  level: number;
  isGolden?: boolean;
  /** The kind of train (Rust's look): the chip's color and word. */
  look?: HypeTrainLookData;
}

/** The hype train glyph the chips wear. */
export const HypeTrainGlyph = ({ className = 'w-2.5 h-2.5' }: { className?: string }) => (
  <svg className={className} viewBox="0 0 15 13" fill="none" aria-hidden>
    <path
      fillRule="evenodd"
      clipRule="evenodd"
      d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z"
      fill="currentColor"
    />
  </svg>
);

/** A stream card's hype train chip: the kind's color (Rust's `badge`) as the
 *  glaze's accent and its word ("HEIST 5"), or the plain purple "LVL 5" and
 *  the golden one, as before kinds existed. */
export const HypeTrainChip = ({
  info,
  glyphClassName,
  style,
  flat = false,
}: {
  info: HypeTrainBadgeInfo;
  glyphClassName?: string;
  style?: CSSProperties;
  flat?: boolean;
}) => {
  const { color, word } = badgeLook(info.look, info.level);
  const name = info.look?.name ?? 'Hype train';
  return (
    <CardChip
      kind={info.isGolden && !color ? 'hype-golden' : 'hype'}
      flat={flat}
      style={{ ...(color ? ({ '--color-accent': color } as CSSProperties) : {}), ...style }}
      aria-label={`${name} level ${info.level}`}
    >
      <HypeTrainGlyph className={glyphClassName} />
      <span>{word}</span>
    </CardChip>
  );
};
