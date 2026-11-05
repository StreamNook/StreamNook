// profileFrame.json holds the numbers of the member card's nine-slice frame
// (the slice insets of the frame art, and the border it is drawn at). The
// desktop card reads them in ProfileFrame.tsx and the phones through Rust.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import frame from './profileFrame.json';

const component = readFileSync(new URL('../components/profile/ProfileFrame.tsx', import.meta.url), 'utf8').replace(/\r\n/g, '\n');

describe('the profile frame numbers', () => {
  it('are the slice and border the card has always drawn', () => {
    expect(frame).toStrictEqual({ slice: [199, 159, 199, 159], border: [18, 14] });
  });

  it('are the only copy: ProfileFrame reads the file and spells no number of its own', () => {
    expect(component).toContain("from '../../utils/profileFrame.json'");
    expect(component).not.toMatch(/199 159|18px 14px/);
  });
});
