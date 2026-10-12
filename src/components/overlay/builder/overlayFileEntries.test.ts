// Run with: npm test
//
// Hidden phrases and command filters in overlay files. Entries saved before the
// builder capped new ones can be long: they travel whole up to
// OVERLAY_FILE_TEXT_ENTRY_MAX, and a longer one is left out and reported. The
// inputs and results match the desktop half's test
// (overlay_file.rs long_list_entries_travel_whole_up_to_the_cap), so a file
// opens the same in the app and on the site.

import { describe, expect, it } from 'vitest';

import { buildOverlayFile, parseOverlayFile, sanitizeOverlayFileStyle } from './profileSync';
import {
  DEFAULT_OVERLAY_STYLE,
  OVERLAY_FILE_TEXT_ENTRY_MAX,
  OVERLAY_STYLE_MAX_CHARS,
  OVERLAY_TEXT_ENTRY_MAX,
  overlayStyleTooLarge,
  type OverlayStyle,
} from '../overlayConfig';

describe('overlay file list entries', () => {
  it('keeps long entries whole up to the cap and reports longer ones', () => {
    const legacy = 'r'.repeat(300);
    const atCap = 'p'.repeat(OVERLAY_FILE_TEXT_ENTRY_MAX);
    const tooLong = 'q'.repeat(OVERLAY_FILE_TEXT_ENTRY_MAX + 1);
    const longCommand = `!${'y'.repeat(OVERLAY_FILE_TEXT_ENTRY_MAX)}`;
    const { style, dropped } = sanitizeOverlayFileStyle({
      hidePhrases: [legacy, atCap, tooLong, '  ', 'Spoiler', 'spoiler', 'tab\there', 7],
      commandFilters: [
        { value: longCommand, mode: 'exact' },
        { value: '!ok', mode: 'exact' },
        { value: ' ', mode: 'prefix' },
        { mode: 'prefix' },
        '!legacy',
      ],
    });
    expect(style.hidePhrases).toEqual([legacy, atCap, 'Spoiler']);
    expect(style.commandFilters).toEqual([{ value: '!ok', mode: 'exact' }]);
    expect(dropped).toEqual(['commandFilters', 'hidePhrases']);

    const clean = sanitizeOverlayFileStyle({ hidePhrases: ['  a  ', '', 'b'], commandFilters: [{ value: '!', mode: 'prefix' }] });
    expect(clean.style.hidePhrases).toEqual(['a', 'b']);
    expect(clean.dropped).toEqual([]);
  });

  it('round-trips an entry longer than the new-entry cap unchanged', () => {
    const legacy = 'x'.repeat(OVERLAY_TEXT_ENTRY_MAX + 100);
    const text = buildOverlayFile('Legacy', { ...DEFAULT_OVERLAY_STYLE, hidePhrases: [legacy] });
    const file = parseOverlayFile(text);
    expect(file.style.hidePhrases).toEqual([legacy]);
    expect(file.dropped).toEqual([]);
  });

  it('measures a style against the server cap the way the server does', () => {
    const style: OverlayStyle = { ...DEFAULT_OVERLAY_STYLE };
    expect(overlayStyleTooLarge(style)).toBe(false);
    const big = { ...style, hidePhrases: Array.from({ length: 100 }, (_, i) => `${i}`.padEnd(OVERLAY_TEXT_ENTRY_MAX, 'x')) };
    expect(JSON.stringify(big).length).toBeGreaterThan(OVERLAY_STYLE_MAX_CHARS);
    expect(overlayStyleTooLarge(big)).toBe(true);
  });
});
