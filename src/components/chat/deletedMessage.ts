import type { CSSProperties } from 'react';

// How a removed message is drawn. The chat row and the settings preview both
// read from here (and ModerationTag), so the preview is the real treatment.
export type DeletedMessageStyle = 'strikethrough' | 'dimmed' | 'italic' | 'keep' | 'hidden';
export const DEFAULT_DELETED_STYLE: DeletedMessageStyle = 'strikethrough';

/** Whether the whole moderated row drops to half opacity. */
export const deletedRowDimmed = (style: DeletedMessageStyle): boolean =>
  style === 'strikethrough' || style === 'dimmed';

/** The text treatment for the moderated message body. */
export const deletedBodyStyle = (style: DeletedMessageStyle): CSSProperties | undefined =>
  style === 'strikethrough'
    ? { textDecoration: 'line-through' }
    : style === 'italic'
      ? { fontStyle: 'italic', color: 'var(--color-text-secondary)' }
      : undefined;
