import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { MISSING_BADGES_PAUSED_TOAST, stopArgs, stopToast } from './dropsStop';

describe('stopArgs', () => {
  it('names the campaign the control shows', () => {
    expect(stopArgs('c-frog')).toEqual({ campaign_id: 'c-frog' });
  });
  it('sends nothing when no campaign is known', () => {
    expect(stopArgs(null)).toEqual({});
    expect(stopArgs('')).toEqual({});
    expect(stopArgs()).toEqual({});
  });
});

describe('stopToast', () => {
  it('says Missing badges is paused only while the switch is on', () => {
    expect(stopToast({ collect_missing_badges: true }, 'Automation stopped')).toBe(
      'Stopped. Missing badges is paused until you resume it or StreamNook restarts.',
    );
    expect(stopToast({ collect_missing_badges: false }, 'Automation stopped')).toBe('Automation stopped');
    expect(stopToast({}, 'Automation stopped')).toBe('Automation stopped');
    expect(stopToast(null, 'Automation stopped')).toBe('Automation stopped');
    expect(MISSING_BADGES_PAUSED_TOAST).not.toMatch(/—/);
  });
});

// Every place the app sends drops.stop names its campaign and words its toast through these.
describe('the Stop routes', () => {
  const read = (p: string) => readFileSync(resolve(__dirname, p), 'utf8').replace(/\r\n/g, '\n');
  for (const file of ['../components/DropsCenter.tsx', '../components/Home.tsx', '../components/ChatWidget.tsx']) {
    it(`${file} sends a campaign and a paused toast`, () => {
      const src = read(file);
      expect(src).toMatch(/action: 'drops\.stop', args: stopArgs\(/);
      expect(src).not.toMatch(/action: 'drops\.stop', args: \{\}/);
      expect(src).toContain('stopToast(');
    });
  }
});
