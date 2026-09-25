/** Deterministic id from a section label, so the "On this page" strip, the
 *  settings search and the command palette can target sections that never
 *  declared one. Explicit ids still win. */
export const sectionIdFromLabel = (label: string): string =>
  `settings-section-${label.toLowerCase().replace(/&/g, 'and').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')}`;
