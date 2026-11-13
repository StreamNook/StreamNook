import { describe, expect, it } from 'vitest';
import fixture from './cheerRow.fixture.json';
import { cheerBody, cheerTier, cheerUnit, leadCheerIndex } from './cheerRow';

type Part = number | string;

const segmentsOf = (message: Part[]) =>
  message.map((part) =>
    typeof part === 'number'
      ? { type: 'cheermote', content: `Cheer${part}`, bits: part }
      : { type: 'text', content: part },
  );

const partsOf = (segments: { type: string; content: string; bits?: number }[]): Part[] =>
  segments.map((seg) => (seg.type === 'cheermote' ? (seg.bits ?? 0) : seg.content));

describe('cheer row rules (shared with the iPhone)', () => {
  it.each(fixture.tiers)('$bits bits is tier $tier in $color', ({ bits, tier, color }) => {
    const got = cheerTier(bits);
    expect(got.tier).toBe(tier);
    expect(got.color).toBe(color);
    expect(got.gemUrl).toBe(fixture.gem.replace('{tier}', tier));
  });

  it.each(fixture.units)('$bits reads "$unit"', ({ bits, unit }) => {
    expect(cheerUnit(bits)).toBe(unit);
  });

  it.each(fixture.messages)('$name', ({ message, lead, body, hasContent }) => {
    const segments = segmentsOf(message);
    const idx = leadCheerIndex(segments);
    expect(idx).toBe(lead);
    const got = cheerBody(segments, idx);
    expect(partsOf(got.body)).toEqual(body);
    expect(got.hasContent).toBe(hasContent);
  });
});
