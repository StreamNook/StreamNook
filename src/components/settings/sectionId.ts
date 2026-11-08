/** Deterministic id from a section label, so the "On this page" strip, the
 *  settings search and the command palette can target sections that never
 *  declared one. Explicit ids still win. */
export const sectionIdFromLabel = (label: string): string =>
  `settings-section-${label.toLowerCase().replace(/&/g, 'and').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')}`;

/** Dispatched (bubbling) on a search or deep-link target that is not on screen,
 *  so a pane that shows one section at a time (the overlay builder's tabs) can
 *  open the section holding it before the jump measures where it is. */
export const SETTINGS_REVEAL_EVENT = 'sn-settings-reveal';
