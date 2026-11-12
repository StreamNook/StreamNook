// Run with: npm test
//
// A settings row that exists only for some accounts carries a need. The search
// and the update-notes links must skip it while the need is unmet, and the
// memos that hold search results must run again when the need flips.

import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { findSettingsEntry, searchSettings, SETTINGS_INDEX } from './searchIndex';
import { setSettingNeed, settingNeedsVersion } from '../../utils/settingNeeds';

const source = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

afterEach(() => setSettingNeed('smiley_style', false));

describe('the Smiley style entry', () => {
  it('names its need', () => {
    const entry = SETTINGS_INDEX.find((e) => e.title === 'Smiley style');
    expect(entry).toMatchObject({ tab: 'Chat', section: 'Emotes', needs: 'smiley_style' });
  });

  it('is left out of the search while the account has no set', () => {
    expect(searchSettings('smiley').map((e) => e.title)).not.toContain('Smiley style');
    expect(searchSettings('robots').map((e) => e.title)).not.toContain('Smiley style');
    expect(searchSettings('smiley', 50, ['Chat']).map((e) => e.title)).not.toContain('Smiley style');
  });

  it('is offered once the need is met, by name and by alias, and goes again when it is not', () => {
    setSettingNeed('smiley_style', true);
    expect(searchSettings('smiley')[0]?.title).toBe('Smiley style');
    expect(searchSettings('monkeys').map((e) => e.title)).toContain('Smiley style');
    expect(searchSettings(':)').map((e) => e.title)).toContain('Smiley style');
    expect(searchSettings('smiley', 50, ['Chat']).map((e) => e.title)).toContain('Smiley style');

    setSettingNeed('smiley_style', false);
    expect(searchSettings('smiley').map((e) => e.title)).not.toContain('Smiley style');
  });

  it('changes the version a memo depends on when the need flips', () => {
    const before = settingNeedsVersion();
    setSettingNeed('smiley_style', true);
    expect(settingNeedsVersion()).not.toBe(before);
  });
});

describe('the update-notes link lookup', () => {
  it('skips a row whose need is unmet', () => {
    expect(findSettingsEntry('Smiley style', 'Chat')).toBeUndefined();
    setSettingNeed('smiley_style', true);
    expect(findSettingsEntry('Smiley style', 'Chat')?.section).toBe('Emotes');
  });

  it('still finds ordinary rows, on the tab named', () => {
    expect(findSettingsEntry('Emoji style', 'Chat')?.tab).toBe('Chat');
    expect(findSettingsEntry('Emoji style', 'Overlay')?.tab).toBe('Overlay');
    expect(findSettingsEntry('Emoji style')).toBeDefined();
  });
});

describe('who reads the need', () => {
  it('has the search results memo depend on the needs version', () => {
    const src = source('./SettingsSearchResults.tsx');
    expect(src).toContain('useSettingNeedsVersion()');
    expect(src).toContain('[provided, query, needsVersion]');
  });

  it('has the MultiChat settings search depend on the needs version', () => {
    const src = source('../multichat/ChatOnlySettingsModal.tsx');
    expect(src).toContain('useSettingNeedsVersion()');
    expect(src).toContain('[query, searching, needsVersion]');
  });

  it('has the update-notes link go through the need-aware lookup', () => {
    const src = source('../SettingsDialog.tsx');
    expect(src).toContain('findSettingsEntry(settingsInitialRow, settingsInitialTab)');
    expect(src).not.toContain('SETTINGS_INDEX.find(');
  });
});
