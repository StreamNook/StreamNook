// Where a phone settings row finds its leading glyph. The table itself lives
// with the phone shell (mobile/screens/settingsIcons), which hands it over here
// when it loads, so the desktop bundle never carries glyphs it never draws.
import { SlidersHorizontal, type LucideIcon } from 'lucide-react';

export type SettingGlyphLookup = (section: string | undefined, title: string) => LucideIcon;

let lookup: SettingGlyphLookup | null = null;

export function setPhoneSettingGlyphs(next: SettingGlyphLookup): void {
  lookup = next;
}

/** The glyph for a row, by its section and title; the generic settings glyph
 *  before a table is handed over, so no phone row is drawn without one. */
export function phoneSettingGlyph(section: string | undefined, title: string): LucideIcon {
  return lookup?.(section, title) ?? SlidersHorizontal;
}
