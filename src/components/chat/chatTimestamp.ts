import type { CSSProperties } from 'react';

// The time at the start of a chat row, shared by the real row and the
// settings preview so both draw it the same way.

/** Leading the name line, on the text's baseline. */
export const INLINE_TIME_STYLE: CSSProperties = { fontSize: '0.78em', marginRight: '0.45em' };

/** On its own line above the message (`chat_design.timestamp_position`
 *  "own_line"), set tight so the time stays close to the line it belongs to. */
export const OWN_LINE_TIME_STYLE: CSSProperties = { fontSize: '0.78em', lineHeight: 1.3 };

/** Whether the stored design puts the time on its own line. Anything else
 *  (including no value) is beside the name, the default. */
export const timestampOnOwnLine = (position: string | undefined): boolean => position === 'own_line';
