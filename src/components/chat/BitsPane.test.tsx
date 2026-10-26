import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { BitsPane } from './BitsPane';
import type { BitsState } from '../../services/bitsService';

const tier = (bits: number) => ({ bits, url: `https://example.test/${bits}.gif`, color: '#9147ff' });
const state = (quick_amounts: number[]): BitsState => ({
  enabled: true,
  cheer_min: 1,
  emote_min: 1,
  pick_min: 1,
  pick_max: 100000,
  quick_amounts,
  balance: 450,
  cheermotes: [
    { prefix: 'Cheer', custom: false, tiers: [tier(1), tier(100), tier(1000)] },
    { prefix: '4Head', custom: false, tiers: [tier(1)] },
  ],
  icons: [tier(1), tier(100)],
});
const sheet = (amounts: number[]) =>
  renderToStaticMarkup(<BitsPane variant="sheet" state={state(amounts)} onInsert={() => {}} />);

describe('BitsPane on the phone sheet', () => {
  it('gives every ready-made amount an equal cell on one row', () => {
    const html = sheet([1, 100, 1000, 5000, 10000, 100000]);
    expect(html).toContain('grid-template-columns:repeat(6, minmax(0, 1fr))');
    for (const label of ['1', '100', '1K', '5K', '10K', '100K']) expect(html).toContain(`>${label}</button>`);
  });

  it('keeps four cells at least, so a short list never stretches one chip', () => {
    expect(sheet([1, 100])).toContain('grid-template-columns:repeat(4, minmax(0, 1fr))');
  });

  it('marks the amount the box holds', () => {
    expect(sheet([1, 100])).toMatch(/aria-pressed="true"[^>]*>1<\/button>/);
  });
});

describe('BitsPane in the desktop menu', () => {
  it('keeps the chips beside the field', () => {
    const html = renderToStaticMarkup(<BitsPane state={state([1, 100])} onInsert={() => {}} />);
    expect(html).not.toContain('grid-template-columns:repeat(');
    expect(html).toContain('w-[6.2ch]');
  });
});
