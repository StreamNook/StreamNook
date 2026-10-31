// The "that press was a hold" mark a card's hold handlers leave for the click
// that follows the lift. Read once: the card's own click and a tag pill on the
// card's band both consume it, so one hold never also counts as a tap. Pure, so
// a node test can drive it.
export interface HoldFlag {
  set: () => void;
  clear: () => void;
  /** Whether the press was a hold, clearing the mark. */
  consume: () => boolean;
}

export function createHoldFlag(): HoldFlag {
  let held = false;
  return {
    set: () => {
      held = true;
    },
    clear: () => {
      held = false;
    },
    consume: () => {
      const was = held;
      held = false;
      return was;
    },
  };
}
