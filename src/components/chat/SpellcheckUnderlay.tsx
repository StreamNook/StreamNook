import React from 'react';

/**
 * Red wavy underlines for misspelled words in a textarea.
 *
 * A textarea can't style its own text, so this is the usual trick: a div that
 * re-renders the same string with the same metrics, laid exactly over the box.
 * The text here is transparent and only the decoration paints, so what shows is
 * a squiggle sitting in the descender space under the real glyphs.
 *
 * It goes ON TOP of the textarea, not behind it — the composer's `glass-input`
 * background is opaque enough to hide anything underneath. `pointer-events-none`
 * keeps clicks, selection and the caret going to the real field.
 *
 * Alignment is fussy in two specific ways, both learned from the reminder-chip
 * backdrop this mirrors:
 *   - the box model has to match, including the textarea's 1px border, or every
 *     line lands a pixel off
 *   - the caller has to feed it the same padding and font the textarea uses
 */

interface SpellcheckUnderlayProps {
  text: string;
  /** [start, end) offsets into `text`, ascending and non-overlapping. */
  ranges: Array<[number, number]>;
  /** The textarea's typography and padding classes, verbatim. Font size and
   *  line height have to match exactly or the squiggles drift line by line. */
  className?: string;
  /** Any geometry the textarea carries via inline style instead of classes. */
  style?: React.CSSProperties;
  /** Scroll sync target — the caller mirrors the textarea's scroll position. */
  innerRef?: React.RefObject<HTMLDivElement | null>;
}

const underlineStyle: React.CSSProperties = {
  // Longhands, not the `text-decoration` shorthand: the shorthand resets
  // text-decoration-color back to currentColor, which here is transparent.
  textDecorationLine: 'underline',
  textDecorationStyle: 'wavy',
  textDecorationColor: 'var(--color-error)',
  // Without this the squiggle breaks around descenders and looks like dashes.
  textDecorationSkipInk: 'none',
  textUnderlineOffset: '2px',
};

export const SpellcheckUnderlay: React.FC<SpellcheckUnderlayProps> = ({
  text,
  ranges,
  className = '',
  style,
  innerRef,
}) => {
  const segments: React.ReactNode[] = [];
  let cursor = 0;

  for (const [start, end] of ranges) {
    if (start > cursor) segments.push(text.slice(cursor, start));
    segments.push(
      <span key={start} style={underlineStyle}>
        {text.slice(start, end)}
      </span>,
    );
    cursor = end;
  }
  if (cursor < text.length) segments.push(text.slice(cursor));

  return (
    <div
      ref={innerRef}
      aria-hidden="true"
      className={`pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words ${className}`}
      style={{
        ...style,
        color: 'transparent',
        // Matches the textarea's `glass-input` border so both boxes lay out
        // their content from the same origin.
        border: '1px solid transparent',
      }}
    >
      {segments}
    </div>
  );
};

export default SpellcheckUnderlay;
