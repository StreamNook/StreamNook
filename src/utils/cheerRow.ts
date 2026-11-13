// The cheer row's rules, one place for every app: the tier an amount falls in
// (its color and Twitch's animated gem), which cheermote the row leads with,
// and the message left under the header. Desktop and Android draw
// `ChatMessage`'s cheer card from these; the iPhone's SNKit `CheerRow` follows
// the same fixture (`cheerRow.fixture.json`).

export type CheerTierName = '1' | '100' | '1000' | '5000' | '10000';

export interface CheerTier {
  tier: CheerTierName;
  /** Twitch's color for the tier: the amount pill's ink and ring. */
  color: string;
  /** Twitch's own animated Cheer gem for the tier. */
  gemUrl: string;
}

const TIERS: readonly [number, CheerTierName, string][] = [
  [10000, '10000', '#ff1f1f'],
  [5000, '5000', '#0099fe'],
  [1000, '1000', '#1db2a6'],
  [100, '100', '#9c3ee8'],
  [1, '1', '#979797'],
];

export function cheerGemUrl(tier: CheerTierName): string {
  return `https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/${tier}/2.gif`;
}

export function cheerTier(bits: number): CheerTier {
  const [, tier, color] = TIERS.find(([floor]) => bits >= floor) ?? TIERS[TIERS.length - 1];
  return { tier, color, gemUrl: cheerGemUrl(tier) };
}

/** "1 bit", "100 bits": the unit after the amount. */
export function cheerUnit(bits: number): 'bit' | 'bits' {
  return bits === 1 ? 'bit' : 'bits';
}

interface CheerSegment {
  type: string;
  content: string;
  bits?: number;
}

/**
 * The cheermote the row leads with: the one carrying the most bits (the first
 * of equals), or -1 when the message has none.
 */
export function leadCheerIndex(segments: readonly CheerSegment[]): number {
  return segments.reduce(
    (best, seg, i) =>
      seg.type === 'cheermote' && (best < 0 || (seg.bits ?? 0) > (segments[best].bits ?? 0)) ? i : best,
    -1,
  );
}

/**
 * The message under the header: every segment but the lead cheermote, the
 * first one's leading space trimmed. `hasContent` is false when nothing but
 * whitespace is left.
 */
export function cheerBody<T extends CheerSegment>(segments: readonly T[], leadIdx: number): { body: T[]; hasContent: boolean } {
  const body = (leadIdx >= 0 ? segments.filter((_, i) => i !== leadIdx) : [...segments]).map((seg, i) =>
    i === 0 && seg.type === 'text' ? { ...seg, content: seg.content.trimStart() } : seg,
  );
  return { body, hasContent: body.some((seg) => seg.type !== 'text' || seg.content.trim() !== '') };
}
