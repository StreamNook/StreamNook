// Run with: npm test
//
// Every Chat entry in the settings search and the Ctrl+K palette has to name a
// section the Chat tab actually renders. A search hit lands on its row when the
// title matches one, and falls back to its section otherwise; an entry naming a
// section that no longer exists lands nowhere, which is exactly how "search for
// pinned messages, click it, nothing happens" shipped. Sections move whenever
// the tab is regrouped, so this reads the rendered tree from source.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { SETTINGS_INDEX } from './searchIndex';
import { sectionIdFromLabel } from './sectionId';

const CHAT_TAB_FILES = [
  'ChatSettings',
  'ImageUploadSettings',
  'HighlightAppearanceSettings',
  'CustomSoundsSettings',
  'HighlightPhrasesSettings',
  'BuiltInHighlightsSettings',
  'UserHighlightsSettings',
  'BadgeHighlightsSettings',
  'UserCommandsSettings',
  'RemindersSettings',
  'UserOverridesSettings',
];

const source = CHAT_TAB_FILES.map((f) =>
  readFileSync(new URL(`./${f}.tsx`, import.meta.url), 'utf8'),
).join('\n');

// `<SettingsSection label="X" ...>` over one or several lines, with any id.
const sections = new Map<string, string>();
for (const m of source.matchAll(/<SettingsSection\b([^>]*?)>/gs)) {
  const label = /label="([^"]+)"/.exec(m[1])?.[1];
  if (!label) continue;
  sections.set(label, /\bid="([^"]+)"/.exec(m[1])?.[1] ?? sectionIdFromLabel(label));
}
const ids = new Set(sections.values());

describe('Chat settings search lands somewhere', () => {
  it('found the rendered sections', () => {
    expect(sections.size).toBeGreaterThan(15);
  });

  const chat = SETTINGS_INDEX.filter((e) => e.tab === 'Chat');
  it.each(chat.map((e) => [e.title, e]))('search entry "%s" names a real section', (_t, e) => {
    expect(sections.has(e.section), `no Chat section labelled "${e.section}"`).toBe(true);
    if (e.sectionId) expect(ids.has(e.sectionId), `no section with id "${e.sectionId}"`).toBe(true);
  });

  const palette = readFileSync(new URL('../../utils/commandPaletteSources.ts', import.meta.url), 'utf8');
  const paletteChat = Array.from(
    palette.matchAll(/\{ tab: 'Chat', section: '([^']+)'(?:, sectionId: '([^']+)')?/g),
  ).map((m) => ({ section: m[1], sectionId: m[2] }));
  it.each(paletteChat.map((e) => [e.section, e]))('palette entry "%s" names a real section', (_s, e) => {
    expect(sections.has(e.section), `no Chat section labelled "${e.section}"`).toBe(true);
    if (e.sectionId) expect(ids.has(e.sectionId), `no section with id "${e.sectionId}"`).toBe(true);
  });
});
