// Run with: npm test
//
// Every setting a viewer can see has to be findable from the settings search,
// and every search entry has to land on something that is still there. Rows
// and sections get renamed and regrouped, and nested settings (SubControl)
// sit inside a parent row, so this reads the rendered tree from source and
// checks both directions: each rendered row and nested line has an index entry
// in the same section (under the same parent row, for a nested one), and each
// entry names a section, row and parent that still render.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { SETTINGS_INDEX, type SettingsIndexEntry } from './searchIndex';

const TAB_FILES: Record<string, string[]> = {
  Player: ['PlayerSettings'],
  Interface: ['InterfaceSettings', 'CompactViewSettings'],
  Chat: [
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
  ],
  Moderation: ['ModerationSettings'],
  Notifications: ['NotificationsSettings'],
  Cache: ['CacheSettings', 'EmotePrefetchSection'],
  Backup: ['BackupSettings'],
  Support: ['SupportSettings'],
  Overlay: ['../overlay/builder/OverlayBuilder'],
};

// A component that draws its own section, with rows passed in as children.
const COMPONENT_SECTIONS: Record<string, string> = { CompactViewSettings: 'Compact View' };

// Never indexed: phone-only settings (search runs only in the desktop Settings
// dialog and the MultiChat modal) and development-build previews.
const UNINDEXED: Record<string, { sections?: string[]; titles?: string[] }> = {
  Chat: { sections: ['Landscape Chat'], titles: ['Vibration'] },
  Player: { titles: ['Background play'] },
  Support: { titles: ['Log export'] },
  Notifications: { titles: ['Gift sub preview', 'Reward preview'] },
};

// Entries that describe a feature rather than one row (how to start a poll,
// where custom sounds go). They land on their section, or on the tab when the
// feature has no section of its own (TOPIC_SECTIONS).
const TOPICS = new Set([
  'Starting a poll or prediction',
  'Custom highlight sounds',
  'Auto message timer',
  'Image host',
  'AutoMod held messages',
  'Update straight from the toast',
  'Join the Discord',
]);
const TOPIC_SECTIONS: Record<string, string[]> = {
  Moderation: ['AutoMod'],
  Support: ['Community Discord'],
  // The overlay builder's profile picker, above its sections.
  Overlay: ['Stream Overlay'],
};

// A row whose title is an expression (`title={row.label}`) is a parent the
// source cannot name; its nested lines match an entry under any parent.
const ANY_PARENT = '*';

interface Rendered {
  section: string;
  title: string;
  parent?: string;
}

/** A row title as search matches it: a title that shows its value
 *  (`Spacing: ${n}px`) is found by the words before the value. */
const titleKey = (raw: string): string => raw.split('${')[0].replace(/[:\s]+$/, '').trim();

function renderedTree(files: string[]): { rows: Rendered[]; sections: Set<string>; source: string } {
  const rows: Rendered[] = [];
  const sections = new Set<string>();
  const sources: string[] = [];
  for (const f of files) {
    const src = readFileSync(new URL(`./${f}.tsx`, import.meta.url), 'utf8');
    sources.push(src);
    // Rows nest two ways: a SubControl inside its row, or whole rows inside a
    // SettingsSubGroup (the overlay builder), which belong to the row above it.
    const re =
      /<SettingsSection\b[^>]*?\blabel="([^"]+)"|<(SettingsRow|SubControl)\s+(?:(?:key|onReset)=\{[^}]*\}\s+)*title=(?:"([^"]*)"|\{`([^`]*)`\}|\{([^`}][^}]*)\})|<(\/?)SettingsSubGroup>|<([A-Z]\w+)\b/g;
    let section = '';
    let lastRow = '';
    let groupParent: string | null = null;
    for (const m of src.matchAll(re)) {
      if (m[1]) {
        section = m[1];
        sections.add(section);
        lastRow = '';
        groupParent = null;
      } else if (m[2] && m[5]) {
        if (m[2] === 'SettingsRow' && groupParent === null) lastRow = ANY_PARENT;
      } else if (m[2]) {
        const title = titleKey(m[3] ?? m[4] ?? '');
        if (!title) continue;
        if (m[2] === 'SettingsRow' && groupParent !== null) {
          rows.push({ section, title, parent: groupParent });
        } else if (m[2] === 'SettingsRow') {
          lastRow = title;
          rows.push({ section, title });
        } else {
          rows.push({ section, title, parent: lastRow });
        }
      } else if (m[6] !== undefined && m[0].endsWith('SettingsSubGroup>')) {
        groupParent = m[6] === '/' ? null : lastRow;
      } else if (m[7] && COMPONENT_SECTIONS[m[7]]) {
        section = COMPONENT_SECTIONS[m[7]];
        sections.add(section);
        lastRow = '';
      }
    }
  }
  return { rows, sections, source: sources.join('\n') };
}

const palette = readFileSync(new URL('../../utils/commandPaletteSources.ts', import.meta.url), 'utf8');

const sameTitle = (entry: string, rendered: string) => entry === rendered;
const sameParent = (entry: string | undefined, rendered: string | undefined) =>
  rendered === ANY_PARENT ? !!entry : (entry ?? '') === (rendered ?? '');

for (const [tab, files] of Object.entries(TAB_FILES)) {
  const { rows, sections, source } = renderedTree(files);
  const skip = UNINDEXED[tab] ?? {};
  const visible = rows.filter(
    (r) => !skip.sections?.includes(r.section) && !skip.titles?.includes(r.title),
  );
  const entries = SETTINGS_INDEX.filter((e) => e.tab === tab);

  describe(`${tab} settings are all searchable`, () => {
    it('found rendered rows', () => {
      expect(visible.length).toBeGreaterThan(0);
    });

    it.each(visible.map((r) => [`${r.section} > ${r.parent ? `${r.parent} > ` : ''}${r.title}`, r] as const))(
      '%s has a search entry',
      (_label, r) => {
        const hit = entries.find(
          (e) =>
            e.section === r.section &&
            sameTitle(e.title, r.title) &&
            sameParent(e.parent, r.parent),
        );
        expect(hit, `no ${tab} search entry for "${r.title}" in "${r.section}"${r.parent ? ` under "${r.parent}"` : ''}`).toBeTruthy();
      },
    );

    it.each(entries.map((e) => [`${e.section} > ${e.parent ? `${e.parent} > ` : ''}${e.title}`, e] as const))(
      'entry %s lands on something rendered',
      (_label, e: SettingsIndexEntry) => {
        if (TOPIC_SECTIONS[tab]?.includes(e.section)) return;
        expect(sections.has(e.section), `no ${tab} section labelled "${e.section}"`).toBe(true);
        if (e.parent) {
          expect(
            rows.some((r) => r.section === e.section && sameParent(e.parent, r.parent) && sameTitle(e.title, r.title)),
            `no "${e.title}" nested under "${e.parent}" in "${e.section}"`,
          ).toBe(true);
          return;
        }
        const isRow = rows.some((r) => r.section === e.section && !r.parent && sameTitle(e.title, r.title));
        // Rows drawn from a list (user card fields, overlay buttons) carry
        // their names as string literals in the same file; a row whose title
        // is a logo plus a word ends its markup with that word.
        const isListed =
          source.includes(`'${e.title}'`) || source.includes(`"${e.title}"`) || source.includes(` ${e.title}</span>`);
        expect(
          isRow || e.title === e.section || isListed || TOPICS.has(e.title),
          `"${e.title}" is not a row in "${e.section}"`,
        ).toBe(true);
      },
    );

    it('no section shows two rows with the same name at the same level', () => {
      const seen = new Set<string>();
      const dupes: string[] = [];
      for (const r of rows) {
        const k = `${r.section}\u0000${r.parent ?? ''}\u0000${r.title}`;
        if (seen.has(k)) dupes.push(`${r.section} > ${r.parent ? `${r.parent} > ` : ''}${r.title}`);
        seen.add(k);
      }
      expect(dupes).toEqual([]);
    });

    const paletteSections = Array.from(
      palette.matchAll(new RegExp(`\\{ tab: '${tab}', section: '([^']+)'`, 'g')),
      (m) => m[1],
    );
    it.each(paletteSections)('palette entry "%s" names a rendered section', (s) => {
      expect(
        sections.has(s) || TOPIC_SECTIONS[tab]?.includes(s),
        `no ${tab} section labelled "${s}"`,
      ).toBe(true);
    });
  });
}
