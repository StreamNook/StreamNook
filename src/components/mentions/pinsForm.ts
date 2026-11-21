// How Rust's slot for the `mention_pins` card reaches the pins block, and how
// the block's drawn heights reach the stack's cache.
import type { PinsForm } from './MentionPins';
import type { StackForm } from '../chat/chatTopStack';

/** The block's form for a slot form: a folded or unplaced card draws nothing
 *  (the host never mounts the block then); anything else the pins do not know
 *  (a strip) is their floor. */
export function pinsFormOf(form: StackForm | null): PinsForm {
  if (form === 'full') return 'full';
  if (form === 'count_line') return 'count_line';
  return 'floor';
}

/** Which of the card's three cached heights a drawn form is. */
export function pinsMeasureForm(form: PinsForm): 'strip' | 'open' | 'line' {
  if (form === 'full') return 'open';
  if (form === 'count_line') return 'line';
  return 'strip';
}
