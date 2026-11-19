// The teaching placeholder's words over an empty message box. A new hint
// (a new chat, or the next one after a message, `useComposerHint`) arrives as
// a slow, soft cross-fade, opacity only, both words in the same spot so
// nothing jumps. The field under it keeps its own placeholder (transparent
// via `textarea[data-hint-layer]`), so screen readers still hear the prompt.
// Shown only at full motion, so the fade always runs.
import { AnimatePresence, motion } from 'framer-motion';
import type { CSSProperties } from 'react';

// A dissolve, even at both ends (the deck's face uses the same).
const FADE = { duration: 0.5, ease: 'easeInOut' } as const;

export function ComposerHintLayer({
  hintKey,
  wide,
  narrow,
  className = '',
  style,
}: {
  /** Changes with each hint: the swap animates on it. */
  hintKey: string;
  wide: string;
  /** The words for a narrow composer row (`.sn-composer-row[data-narrow]`);
   *  absent where the caller already picked words that fit. */
  narrow?: string;
  /** The field's text box: font, padding and placeholder ink. */
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <div aria-hidden className={`pointer-events-none absolute inset-0 z-[1] overflow-hidden ${className}`} style={style}>
      <div className="relative h-full">
        <AnimatePresence initial={false}>
          <motion.span
            key={hintKey}
            className="absolute inset-x-0 top-0 truncate whitespace-nowrap"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={FADE}
          >
            {narrow === undefined ? (
              wide
            ) : (
              <>
                <span className="sn-hint-wide">{wide}</span>
                <span className="sn-hint-narrow">{narrow}</span>
              </>
            )}
          </motion.span>
        </AnimatePresence>
      </div>
    </div>
  );
}
