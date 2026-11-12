// The phone settings catalog lists Smiley style under Chat > Emotes, and only
// while Rust says the account has a style to pick.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

interface Row {
  page: string;
  section: string;
  title: string;
  needs?: string;
  paths?: string[];
}

const catalog = JSON.parse(
  readFileSync(resolve(__dirname, '../../../src-tauri/src/services/settings_catalog.json'), 'utf8'),
) as { rows: Row[] };

describe('Smiley style catalog row', () => {
  it('sits in Chat > Emotes and needs the smiley style capability', () => {
    const rows = catalog.rows.filter((r) => r.title === 'Smiley style');
    expect(rows).toHaveLength(1);
    expect(rows[0].page).toBe('chat');
    expect(rows[0].section).toBe('Emotes');
    expect(rows[0].needs).toBe('smiley_style');
    expect(rows[0].paths ?? []).toEqual([]);
  });

  it('comes right after Emoji style', () => {
    const titles = catalog.rows.filter((r) => r.page === 'chat' && r.section === 'Emotes').map((r) => r.title);
    expect(titles[titles.indexOf('Smiley style') - 1]).toBe('Emoji style');
  });
});
