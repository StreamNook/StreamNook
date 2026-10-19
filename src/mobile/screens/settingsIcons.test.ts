// Every settings row the phone draws leads with a glyph from the table, never
// the generic stand-in. The settings catalog Rust keeps lists every row by
// page, section and title, so a row added there without a glyph fails here.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { FALLBACK_SETTING_ICON, settingIcon, settingIconFor, settingRowName } from './settingsIcons';

interface Row {
  page: string;
  section: string;
  title: string;
  nested?: boolean;
}

const catalog = JSON.parse(
  readFileSync(resolve(__dirname, '../../../src-tauri/src/services/settings_catalog.json'), 'utf8'),
) as { rows: Row[] };

describe('phone settings glyphs', () => {
  it('names a glyph for every top-level catalog row', () => {
    const missing = catalog.rows
      .filter((row) => !row.nested)
      .filter((row) => settingIconFor(row.section, row.title) === null)
      .map((row) => `${row.page} › ${row.section} › ${row.title}`);
    expect(missing).toEqual([]);
  });

  it('files a title that shows its value under its name', () => {
    expect(settingRowName('Text size: 14px')).toBe('Text size');
    expect(settingRowName('16:9 window')).toBe('16:9 window');
    expect(settingIconFor('Message Layout', 'Text size: 14px')).toBe(settingIconFor(undefined, 'Text size'));
  });

  it('lets a section own a short name that repeats', () => {
    expect(settingIconFor('Links', 'Style')).not.toBe(settingIconFor('Chat Events', 'Style'));
    expect(settingIconFor('Emotes', 'Spacing')).not.toBe(settingIconFor('Message Layout', 'Spacing'));
  });

  it('falls back to the generic glyph rather than none', () => {
    expect(settingIconFor(undefined, 'Not a real setting')).toBeNull();
    expect(settingIcon(undefined, 'Not a real setting')).toBe(FALLBACK_SETTING_ICON);
  });
});
